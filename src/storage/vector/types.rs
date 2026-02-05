use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vector {
    pub id: u128,
    pub data: Vec<f64>,
    pub level: usize,
}

impl Vector {
    pub fn new(id: u128, data: Vec<f64>, level: usize) -> Self {
        Self { id, data, level }
    }

    /// Calculate Euclidean distance between this vector and another
    pub fn distance(&self, other: &Self) -> f64 {
        self.distance_raw(&other.data)
    }

    pub fn distance_raw(&self, other_data: &[f64]) -> f64 {
        self.data.iter()
            .zip(other_data.iter())
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            .sqrt()
    }

    pub fn cosine_distance(&self, other_data: &[f64]) -> f64 {
        let dot_product: f64 = self.data.iter().zip(other_data.iter()).map(|(a, b)| a * b).sum();
        let norm_a: f64 = self.data.iter().map(|a| a.powi(2)).sum::<f64>().sqrt();
        let norm_b: f64 = other_data.iter().map(|b| b.powi(2)).sum::<f64>().sqrt();
        
        if norm_a == 0.0 || norm_b == 0.0 {
            return 1.0; // Max distance if zero vector
        }

        let similarity = dot_product / (norm_a * norm_b);
        // Distance = 1 - Similarity. range [0, 2] usually.
        // HNSW needs non-negative distance.
        (1.0 - similarity).max(0.0)
    }
}
