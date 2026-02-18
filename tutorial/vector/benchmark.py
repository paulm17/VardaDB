#!/usr/bin/env python3
"""
VardaDB Search Benchmark Script
Tests vector search, hybrid search, and BM25 text search performance.

Requirements:
    pip install httpx numpy

Usage:
    python benchmark.py --url http://localhost:4000/graphql --iterations 100
"""

import httpx
import numpy as np
import time
import argparse

VECTOR_DIM = 128

def benchmark_vector_search(url: str, iterations: int):
    """Benchmark pure vector search."""
    query = """
    query VectorSearch($vec: [Float!]!, $k: Int!) {
        search(vector: $vec, k: $k) { uid distance }
    }
    """
    
    latencies = []
    results_counts = []
    
    print(f"Running {iterations} vector search queries...")
    for i in range(iterations):
        vec = np.random.randn(VECTOR_DIM).tolist()
        start = time.perf_counter()
        resp = httpx.post(url, json={
            "query": query, 
            "variables": {"vec": vec, "k": 10}
        }, timeout=30.0)
        latency = (time.perf_counter() - start) * 1000
        latencies.append(latency)
        
        data = resp.json()
        if "data" in data and data["data"]["search"]:
            results_counts.append(len(data["data"]["search"]))
    
    print(f"\nVector Search (k=10):")
    print(f"  Queries:     {iterations}")
    print(f"  Avg Results: {np.mean(results_counts):.1f}")
    print(f"  p50 Latency: {np.percentile(latencies, 50):.2f}ms")
    print(f"  p95 Latency: {np.percentile(latencies, 95):.2f}ms")
    print(f"  p99 Latency: {np.percentile(latencies, 99):.2f}ms")
    print(f"  Max Latency: {max(latencies):.2f}ms")
    return latencies

def benchmark_hybrid_search(url: str, iterations: int):
    """Benchmark hybrid (BM25 + vector) search."""
    query = """
    query HybridSearch($text: String!, $field: String!, $vec: [Float!]!, $k: Int!) {
        hybridSearch(text: $text, field: $field, vector: $vec, k: $k) { uid distance }
    }
    """
    
    keywords = ["wireless", "comfortable", "premium", "budget", "compact", 
                "durable", "lightweight", "professional", "portable", "quality"]
    latencies = []
    results_counts = []
    
    print(f"\nRunning {iterations} hybrid search queries...")
    for i in range(iterations):
        vec = np.random.randn(VECTOR_DIM).tolist()
        start = time.perf_counter()
        resp = httpx.post(url, json={
            "query": query,
            "variables": {
                "text": keywords[i % len(keywords)],
                "field": "description",
                "vec": vec,
                "k": 10
            }
        }, timeout=30.0)
        latency = (time.perf_counter() - start) * 1000
        latencies.append(latency)
        
        data = resp.json()
        if "data" in data and data["data"]["hybridSearch"]:
            results_counts.append(len(data["data"]["hybridSearch"]))
    
    print(f"\nHybrid Search (k=10):")
    print(f"  Queries:     {iterations}")
    print(f"  Avg Results: {np.mean(results_counts) if results_counts else 0:.1f}")
    print(f"  p50 Latency: {np.percentile(latencies, 50):.2f}ms")
    print(f"  p95 Latency: {np.percentile(latencies, 95):.2f}ms")
    print(f"  p99 Latency: {np.percentile(latencies, 99):.2f}ms")
    print(f"  Max Latency: {max(latencies):.2f}ms")
    return latencies

def benchmark_traversal(url: str, iterations: int):
    """Benchmark graph traversal queries."""
    query = """
    query GraphTraversal {
        queryCategory(first: 3) {
            name
            products(first: 5) {
                name
                price
                reviews(first: 3) {
                    rating
                    author {
                        username
                    }
                }
            }
        }
    }
    """
    
    latencies = []
    
    print(f"\nRunning {iterations} graph traversal queries...")
    for i in range(iterations):
        start = time.perf_counter()
        resp = httpx.post(url, json={"query": query}, timeout=30.0)
        latency = (time.perf_counter() - start) * 1000
        latencies.append(latency)
    
    print(f"\nGraph Traversal (3-hop):")
    print(f"  Queries:     {iterations}")
    print(f"  p50 Latency: {np.percentile(latencies, 50):.2f}ms")
    print(f"  p95 Latency: {np.percentile(latencies, 95):.2f}ms")
    print(f"  p99 Latency: {np.percentile(latencies, 99):.2f}ms")
    print(f"  Max Latency: {max(latencies):.2f}ms")
    return latencies

def main():
    parser = argparse.ArgumentParser(description="Benchmark VardaDB search performance")
    parser.add_argument("--url", default="http://localhost:4000/graphql", help="GraphQL endpoint")
    parser.add_argument("--iterations", type=int, default=100, help="Number of iterations")
    args = parser.parse_args()
    
    print("=" * 60)
    print("VardaDB Search Benchmark")
    print("=" * 60)
    print(f"Endpoint: {args.url}")
    print(f"Iterations: {args.iterations}")
    print(f"Vector Dimension: {VECTOR_DIM}")
    print("=" * 60)
    
    try:
        vec_latencies = benchmark_vector_search(args.url, args.iterations)
        hybrid_latencies = benchmark_hybrid_search(args.url, args.iterations)
        trav_latencies = benchmark_traversal(args.url, args.iterations)
        
        print("\n" + "=" * 60)
        print("Summary")
        print("=" * 60)
        print(f"Vector Search p50:    {np.percentile(vec_latencies, 50):.2f}ms")
        print(f"Hybrid Search p50:    {np.percentile(hybrid_latencies, 50):.2f}ms")
        print(f"Graph Traversal p50:  {np.percentile(trav_latencies, 50):.2f}ms")
        print("=" * 60)
        
    except httpx.ConnectError:
        print(f"\nError: Could not connect to {args.url}")
        print("Make sure VardaDB is running and the URL is correct.")
    except Exception as e:
        print(f"\nError: {e}")

if __name__ == "__main__":
    main()
