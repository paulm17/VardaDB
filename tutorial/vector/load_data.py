#!/usr/bin/env python3
"""
VardaDB E-Commerce Data Loader
Generates thousands of records with vectors and relationships.

Requirements:
    pip install httpx numpy faker
    
Usage:
    python load_data.py --url http://localhost:4000/graphql --products 1000 --reviews 5000
"""

import httpx
import numpy as np
from faker import Faker
import argparse
import random
import time

fake = Faker()

# Configuration
VECTOR_DIM = 128  # Embedding dimension
CATEGORIES = ["Electronics", "Clothing", "Books", "Home", "Sports", "Toys", "Food", "Beauty"]
CITIES = [
    ("New York", "USA"), ("Los Angeles", "USA"), ("London", "UK"),
    ("Paris", "France"), ("Tokyo", "Japan"), ("Sydney", "Australia"),
    ("Berlin", "Germany"), ("Toronto", "Canada")
]

def generate_embedding(seed_text: str) -> list:
    """Generate deterministic pseudo-embedding from text."""
    np.random.seed(hash(seed_text) % (2**32))
    vec = np.random.randn(VECTOR_DIM)
    vec = vec / np.linalg.norm(vec)  # Normalize
    return vec.tolist()

def gql_request(url: str, query: str, variables: dict = None) -> dict:
    """Execute GraphQL request."""
    payload = {"query": query}
    if variables:
        payload["variables"] = variables
    resp = httpx.post(url, json=payload, timeout=30.0)
    resp.raise_for_status()
    return resp.json()

class DataLoader:
    def __init__(self, url: str):
        self.url = url
        self.category_ids = {}
        self.location_ids = {}
        self.store_ids = []
        self.user_ids = []
        self.product_ids = []
    
    def create_categories(self):
        """Create category nodes."""
        print("Creating categories...")
        for cat in CATEGORIES:
            mutation = '''
            mutation CreateCategory($name: String!) {
                createCategory(input: { name: $name }) { id }
            }
            '''
            result = gql_request(self.url, mutation, {"name": cat})
            if "data" in result and result["data"]["createCategory"]:
                self.category_ids[cat] = result["data"]["createCategory"]["id"]
        print(f"  Created {len(self.category_ids)} categories")
    
    def create_locations(self):
        """Create location nodes."""
        print("Creating locations...")
        for city, country in CITIES:
            mutation = '''
            mutation CreateLocation($city: String!, $country: String!) {
                createLocation(input: { city: $city, country: $country }) { id }
            }
            '''
            result = gql_request(self.url, mutation, {"city": city, "country": country})
            if "data" in result and result["data"]["createLocation"]:
                self.location_ids[(city, country)] = result["data"]["createLocation"]["id"]
        print(f"  Created {len(self.location_ids)} locations")
    
    def create_stores(self, count: int = 50):
        """Create store nodes linked to locations."""
        print(f"Creating {count} stores...")
        for i in range(count):
            loc = random.choice(list(self.location_ids.keys()))
            mutation = '''
            mutation CreateStore($name: String!, $rating: Float!, $location: String!) {
                createStore(input: { 
                    name: $name, 
                    rating: $rating,
                    location: $location
                }) { id }
            }
            '''
            result = gql_request(self.url, mutation, {
                "name": fake.company(),
                "rating": round(random.uniform(3.0, 5.0), 1),
                "location": self.location_ids[loc]
            })
            if "data" in result and result["data"]["createStore"]:
                self.store_ids.append(result["data"]["createStore"]["id"])
        print(f"  Created {len(self.store_ids)} stores")
    
    def create_users(self, count: int = 200):
        """Create user nodes."""
        print(f"Creating {count} users...")
        for i in range(count):
            mutation = '''
            mutation CreateUser($username: String!, $email: String!) {
                createUser(input: { username: $username, email: $email }) { id }
            }
            '''
            result = gql_request(self.url, mutation, {
                "username": f"{fake.user_name()}_{i}",
                "email": f"user{i}_{fake.email()}"
            })
            if "data" in result and result["data"]["createUser"]:
                self.user_ids.append(result["data"]["createUser"]["id"])
        print(f"  Created {len(self.user_ids)} users")
    
    def create_products(self, count: int = 1000):
        """Create product nodes with vectors."""
        print(f"Creating {count} products with embeddings...")
        start = time.time()
        
        for i in range(count):
            cat = random.choice(CATEGORIES)
            name = f"{fake.word().title()} {fake.word().title()} {cat}"
            desc = fake.paragraph(nb_sentences=3)
            
            mutation = '''
            mutation CreateProduct($name: String!, $description: String, $price: Float!, 
                                   $embedding: [Float!]!, $category: String!, $store: String!) {
                createProduct(input: {
                    name: $name,
                    description: $description,
                    price: $price,
                    embedding: $embedding,
                    category: $category,
                    store: $store
                }) { id }
            }
            '''
            result = gql_request(self.url, mutation, {
                "name": name,
                "description": desc,
                "price": round(random.uniform(9.99, 999.99), 2),
                "embedding": generate_embedding(name + desc),
                "category": self.category_ids[cat],
                "store": random.choice(self.store_ids)
            })
            if "data" in result and result["data"]["createProduct"]:
                self.product_ids.append(result["data"]["createProduct"]["id"])
            
            if (i + 1) % 100 == 0:
                elapsed = time.time() - start
                rate = (i + 1) / elapsed
                print(f"  Progress: {i+1}/{count} ({rate:.1f} products/sec)")
        
        print(f"  Created {len(self.product_ids)} products")
    
    def create_reviews(self, count: int = 5000):
        """Create review nodes with vectors, linking users and products."""
        print(f"Creating {count} reviews with embeddings...")
        start = time.time()
        created = 0
        
        for i in range(count):
            title = fake.sentence(nb_words=6)
            content = fake.paragraph(nb_sentences=4)
            
            mutation = '''
            mutation CreateReview($title: String, $content: String!, $rating: Int!,
                                  $embedding: [Float!]!, $author: String!, $product: String!) {
                createReview(input: {
                    title: $title,
                    content: $content,
                    rating: $rating,
                    embedding: $embedding,
                    author: $author,
                    product: $product
                }) { id }
            }
            '''
            result = gql_request(self.url, mutation, {
                "title": title,
                "content": content,
                "rating": random.randint(1, 5),
                "embedding": generate_embedding(title + content),
                "author": random.choice(self.user_ids),
                "product": random.choice(self.product_ids)
            })
            
            if "data" in result and result["data"]["createReview"]:
                created += 1
            
            if (i + 1) % 500 == 0:
                elapsed = time.time() - start
                rate = (i + 1) / elapsed
                print(f"  Progress: {i+1}/{count} ({rate:.1f} reviews/sec)")
        
        print(f"  Created {created} reviews")

    def run(self, products: int, reviews: int):
        """Run the full data loading pipeline."""
        print("=" * 60)
        print("VardaDB E-Commerce Data Loader")
        print("=" * 60)
        
        self.create_categories()
        self.create_locations()
        self.create_stores(count=50)
        self.create_users(count=200)
        self.create_products(count=products)
        self.create_reviews(count=reviews)
        
        print("=" * 60)
        print("Data loading complete!")
        print(f"  Categories: {len(self.category_ids)}")
        print(f"  Locations:  {len(self.location_ids)}")
        print(f"  Stores:     {len(self.store_ids)}")
        print(f"  Users:      {len(self.user_ids)}")
        print(f"  Products:   {len(self.product_ids)}")
        print(f"  Reviews:    {reviews}")
        print("=" * 60)

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Load e-commerce data into VardaDB")
    parser.add_argument("--url", default="http://localhost:4000/graphql", help="GraphQL endpoint")
    parser.add_argument("--products", type=int, default=1000, help="Number of products")
    parser.add_argument("--reviews", type=int, default=5000, help="Number of reviews")
    args = parser.parse_args()
    
    loader = DataLoader(args.url)
    loader.run(products=args.products, reviews=args.reviews)
