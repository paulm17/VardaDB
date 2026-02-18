#!/usr/bin/env python3
"""
VardaDB Test Queries Script
Runs various queries to test all vector engine functionality.

Usage:
    python test_queries.py --url http://localhost:4000/graphql
"""

import httpx
import numpy as np
import argparse
import json

VECTOR_DIM = 128

def gql(url: str, query: str, variables: dict = None) -> dict:
    """Execute GraphQL request."""
    payload = {"query": query}
    if variables:
        payload["variables"] = variables
    resp = httpx.post(url, json=payload, timeout=30.0)
    return resp.json()

def test_vector_search(url: str):
    """Test pure vector search."""
    print("\n" + "=" * 50)
    print("TEST: Vector Search")
    print("=" * 50)
    
    vec = np.random.randn(VECTOR_DIM).tolist()
    query = """
    query VectorSearch($vec: [Float!]!, $k: Int!) {
        search(vector: $vec, k: $k) { uid distance }
    }
    """
    result = gql(url, query, {"vec": vec, "k": 5})
    
    if "errors" in result:
        print(f"❌ FAILED: {result['errors']}")
        return False
    
    results = result.get("data", {}).get("search", [])
    print(f"✅ PASSED: Found {len(results)} results")
    for r in results[:3]:
        print(f"   - UID: {r['uid']}, Distance: {r['distance']:.4f}")
    return True

def test_hybrid_search(url: str):
    """Test hybrid (BM25 + vector) search."""
    print("\n" + "=" * 50)
    print("TEST: Hybrid Search (BM25 + Vector)")
    print("=" * 50)
    
    vec = np.random.randn(VECTOR_DIM).tolist()
    query = """
    query HybridSearch($text: String!, $field: String!, $vec: [Float!]!, $k: Int!) {
        hybridSearch(text: $text, field: $field, vector: $vec, k: $k) { uid distance }
    }
    """
    result = gql(url, query, {
        "text": "quality product",
        "field": "description", 
        "vec": vec, 
        "k": 5
    })
    
    if "errors" in result:
        print(f"❌ FAILED: {result['errors']}")
        return False
    
    results = result.get("data", {}).get("hybridSearch", [])
    print(f"✅ PASSED: Found {len(results)} results")
    for r in results[:3]:
        print(f"   - UID: {r['uid']}, Distance: {r['distance']:.4f}")
    return True

def test_graph_traversal(url: str):
    """Test graph traversal (forward and reverse)."""
    print("\n" + "=" * 50)
    print("TEST: Graph Traversal")
    print("=" * 50)
    
    query = """
    query GraphTraversal {
        queryCategory(first: 2) {
            name
            products(first: 2) {
                name
                price
                store {
                    name
                    location {
                        city
                        country
                    }
                }
                reviews(first: 2) {
                    rating
                    author {
                        username
                    }
                }
            }
        }
    }
    """
    result = gql(url, query)
    
    if "errors" in result:
        print(f"❌ FAILED: {result['errors']}")
        return False
    
    categories = result.get("data", {}).get("queryCategory", [])
    print(f"✅ PASSED: Traversed {len(categories)} categories")
    for cat in categories:
        prods = cat.get("products", [])
        print(f"   - {cat['name']}: {len(prods)} products")
    return True

def test_text_search_bm25(url: str):
    """Test BM25 text search."""
    print("\n" + "=" * 50)
    print("TEST: BM25 Text Search")
    print("=" * 50)
    
    query = """
    query TextSearch {
        queryProduct(filter: {
            name: { alloftext: "Electronics" }
        }, first: 5) {
            id
            name
            price
        }
    }
    """
    result = gql(url, query)
    
    if "errors" in result:
        print(f"❌ FAILED: {result['errors']}")
        return False
    
    products = result.get("data", {}).get("queryProduct", [])
    print(f"✅ PASSED: Found {len(products)} products matching 'Electronics'")
    for p in products[:3]:
        print(f"   - {p['name']}: ${p['price']:.2f}")
    return True

def test_create_update_delete(url: str):
    """Test CRUD operations with vectors."""
    print("\n" + "=" * 50)
    print("TEST: Create/Update/Delete with Vector")
    print("=" * 50)
    
    # Get a category ID first
    cat_query = "query { queryCategory(first: 1) { id } }"
    cat_result = gql(url, cat_query)
    if not cat_result.get("data", {}).get("queryCategory"):
        print("❌ FAILED: No categories found. Load data first.")
        return False
    cat_id = cat_result["data"]["queryCategory"][0]["id"]
    
    # Get a store ID
    store_query = "query { queryStore(first: 1) { id } }"
    store_result = gql(url, store_query)
    if not store_result.get("data", {}).get("queryStore"):
        print("❌ FAILED: No stores found. Load data first.")
        return False
    store_id = store_result["data"]["queryStore"][0]["id"]
    
    # CREATE
    vec = np.random.randn(VECTOR_DIM).tolist()
    create_mutation = """
    mutation CreateProduct($name: String!, $price: Float!, $embedding: [Float!]!, 
                           $category: String!, $store: String!) {
        createProduct(input: {
            name: $name,
            description: "Test product for CRUD testing",
            price: $price,
            embedding: $embedding,
            category: $category,
            store: $store
        }) { id name }
    }
    """
    create_result = gql(url, create_mutation, {
        "name": "CRUD Test Product",
        "price": 99.99,
        "embedding": vec,
        "category": cat_id,
        "store": store_id
    })
    
    if "errors" in create_result:
        print(f"❌ CREATE FAILED: {create_result['errors']}")
        return False
    
    product_id = create_result["data"]["createProduct"]["id"]
    print(f"✅ CREATE: Created product {product_id}")
    
    # UPDATE
    new_vec = np.random.randn(VECTOR_DIM).tolist()
    update_mutation = """
    mutation UpdateProduct($id: ID!, $embedding: [Float!]!) {
        updateProduct(id: $id, input: { embedding: $embedding })
    }
    """
    update_result = gql(url, update_mutation, {"id": product_id, "embedding": new_vec})
    
    if "errors" in update_result:
        print(f"❌ UPDATE FAILED: {update_result['errors']}")
        return False
    print(f"✅ UPDATE: Updated product embedding")
    
    # SEARCH (verify it appears)
    search_query = """
    query Search($vec: [Float!]!) {
        search(vector: $vec, k: 20) { uid }
    }
    """
    search_result = gql(url, search_query, {"vec": new_vec})
    uids = [r["uid"] for r in search_result.get("data", {}).get("search", [])]
    if product_id in uids:
        print(f"✅ SEARCH: Product found in vector search results")
    else:
        print(f"⚠️  SEARCH: Product not in top 20 results (may be expected)")
    
    # DELETE
    delete_mutation = """
    mutation DeleteProduct($id: ID!) {
        deleteProduct(id: $id)
    }
    """
    delete_result = gql(url, delete_mutation, {"id": product_id})
    
    if "errors" in delete_result:
        print(f"❌ DELETE FAILED: {delete_result['errors']}")
        return False
    print(f"✅ DELETE: Deleted product")
    
    # VERIFY (should not appear in search)
    search_result2 = gql(url, search_query, {"vec": new_vec})
    uids2 = [r["uid"] for r in search_result2.get("data", {}).get("search", [])]
    if product_id not in uids2:
        print(f"✅ VERIFY: Deleted product no longer in search results")
    else:
        print(f"❌ VERIFY FAILED: Deleted product still appears!")
        return False
    
    return True

def test_inverse_relationships(url: str):
    """Test bidirectional edge traversal."""
    print("\n" + "=" * 50)
    print("TEST: Inverse Relationships (@hasInverse)")
    print("=" * 50)
    
    # Forward: User -> Reviews
    forward_query = """
    query ForwardTraversal {
        queryUser(first: 1) {
            username
            reviews(first: 3) {
                rating
                product {
                    name
                }
            }
        }
    }
    """
    forward_result = gql(url, forward_query)
    
    if "errors" in forward_result:
        print(f"❌ FORWARD FAILED: {forward_result['errors']}")
        return False
    
    users = forward_result.get("data", {}).get("queryUser", [])
    if users:
        reviews = users[0].get("reviews", [])
        print(f"✅ FORWARD: User has {len(reviews)} reviews")
    
    # Reverse: Product -> Reviews -> Authors
    reverse_query = """
    query ReverseTraversal {
        queryProduct(first: 1) {
            name
            reviews(first: 3) {
                rating
                author {
                    username
                }
            }
        }
    }
    """
    reverse_result = gql(url, reverse_query)
    
    if "errors" in reverse_result:
        print(f"❌ REVERSE FAILED: {reverse_result['errors']}")
        return False
    
    products = reverse_result.get("data", {}).get("queryProduct", [])
    if products:
        reviews = products[0].get("reviews", [])
        print(f"✅ REVERSE: Product has {len(reviews)} reviews with authors")
    
    return True

def main():
    parser = argparse.ArgumentParser(description="Test VardaDB vector functionality")
    parser.add_argument("--url", default="http://localhost:4000/graphql", help="GraphQL endpoint")
    args = parser.parse_args()
    
    print("=" * 50)
    print("VardaDB Vector Engine Test Suite")
    print("=" * 50)
    print(f"Endpoint: {args.url}")
    
    tests = [
        ("Vector Search", test_vector_search),
        ("Hybrid Search", test_hybrid_search),
        ("Graph Traversal", test_graph_traversal),
        ("BM25 Text Search", test_text_search_bm25),
        ("Inverse Relationships", test_inverse_relationships),
        ("CRUD Operations", test_create_update_delete),
    ]
    
    results = []
    for name, test_fn in tests:
        try:
            passed = test_fn(args.url)
            results.append((name, passed))
        except Exception as e:
            print(f"❌ {name} EXCEPTION: {e}")
            results.append((name, False))
    
    print("\n" + "=" * 50)
    print("SUMMARY")
    print("=" * 50)
    passed = sum(1 for _, p in results if p)
    total = len(results)
    for name, p in results:
        status = "✅ PASS" if p else "❌ FAIL"
        print(f"  {status}: {name}")
    print(f"\nTotal: {passed}/{total} tests passed")
    print("=" * 50)

if __name__ == "__main__":
    main()
