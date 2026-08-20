//! Vectors, and what can honestly be said about comparing them.
//!
//! Every embedding here is L2-normalized on construction, which makes cosine
//! similarity a dot product and keeps similarity in a range a person can reason
//! about. Vectors are stored as little-endian `f32`, because a local database
//! is never read by a different machine than wrote it — and if that ever
//! changes, the fixed byte order means it still works.

use serde::{Deserialize, Serialize};

/// How much a provider's vectors actually capture.
///
/// This exists because the difference matters and is easy to hide. A hashed
/// bag-of-features embedding finds text that *shares words*; a trained model
/// finds text that *means the same thing*. Both are useful; calling the first
/// one semantic would be a lie, and a user deciding whether to trust a result
/// deserves to know which they have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fidelity {
    /// Similarity comes from shared words and word fragments. Robust to
    /// spelling and morphology; blind to synonyms and paraphrase.
    Lexical,
    /// Similarity comes from a trained model, and tracks meaning.
    Semantic,
}

impl Fidelity {
    pub fn as_str(self) -> &'static str {
        match self {
            Fidelity::Lexical => "lexical",
            Fidelity::Semantic => "semantic",
        }
    }

    /// A sentence to show a person next to a similarity number.
    pub fn caveat(self) -> &'static str {
        match self {
            Fidelity::Lexical => {
                "similarity is lexical: it finds shared wording, not shared meaning"
            }
            Fidelity::Semantic => "similarity comes from a local model",
        }
    }
}

/// A unit-length vector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Embedding {
    values: Vec<f32>,
}

impl Embedding {
    /// Build an embedding, normalizing to unit length.
    ///
    /// A vector of all zeros — which is what embedding empty or unrecognisable
    /// input produces — stays zero rather than becoming `NaN`. Its similarity
    /// to everything is then zero, which is the correct answer.
    pub fn new(mut values: Vec<f32>) -> Self {
        let norm = values.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            for value in &mut values {
                *value /= norm;
            }
        }
        Embedding { values }
    }

    /// The number of dimensions.
    pub fn dimensions(&self) -> usize {
        self.values.len()
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }

    /// Whether this vector carries no signal at all.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty() || self.values.iter().all(|value| *value == 0.0)
    }

    /// Cosine similarity, in `-1..=1`.
    ///
    /// Vectors of different lengths are not comparable, and pretending
    /// otherwise by padding would produce a plausible-looking number from two
    /// unrelated providers. Zero says "no idea", which is true.
    pub fn similarity(&self, other: &Embedding) -> f32 {
        if self.values.len() != other.values.len() {
            return 0.0;
        }

        self.values
            .iter()
            .zip(&other.values)
            .map(|(left, right)| left * right)
            .sum::<f32>()
            .clamp(-1.0, 1.0)
    }

    /// Encode for storage.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.values.len() * 4);
        for value in &self.values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    /// Decode from storage.
    ///
    /// Returns `None` for a length that is not a whole number of `f32`s, which
    /// is what a truncated or corrupt row looks like.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() % 4 != 0 {
            return None;
        }

        let values = bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect::<Vec<f32>>();

        // Already normalized when it was written; re-normalizing would be a
        // no-op that quietly repairs a vector that should be rejected.
        if values.iter().any(|value| !value.is_finite()) {
            return None;
        }
        Some(Embedding { values })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_are_unit_length() {
        let embedding = Embedding::new(vec![3.0, 4.0]);
        let norm: f32 = embedding.values().iter().map(|v| v * v).sum::<f32>().sqrt();

        assert!((norm - 1.0).abs() < 1e-6, "{norm}");
        assert!((embedding.values()[0] - 0.6).abs() < 1e-6);
    }

    #[test]
    fn a_zero_vector_stays_zero_rather_than_becoming_nan() {
        let embedding = Embedding::new(vec![0.0, 0.0, 0.0]);

        assert!(embedding.is_empty());
        assert!(embedding.values().iter().all(|value| value.is_finite()));
        assert_eq!(embedding.similarity(&embedding), 0.0);
    }

    #[test]
    fn identical_vectors_are_maximally_similar() {
        let embedding = Embedding::new(vec![1.0, 2.0, 3.0]);
        assert!((embedding.similarity(&embedding) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn opposite_vectors_are_maximally_dissimilar() {
        let left = Embedding::new(vec![1.0, 0.0]);
        let right = Embedding::new(vec![-1.0, 0.0]);

        assert!((left.similarity(&right) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn orthogonal_vectors_share_nothing() {
        let left = Embedding::new(vec![1.0, 0.0]);
        let right = Embedding::new(vec![0.0, 1.0]);

        assert!(left.similarity(&right).abs() < 1e-6);
    }

    #[test]
    fn vectors_from_different_providers_are_not_compared() {
        let short = Embedding::new(vec![1.0, 0.0]);
        let long = Embedding::new(vec![1.0, 0.0, 0.0, 0.0]);

        assert_eq!(
            short.similarity(&long),
            0.0,
            "padding two providers together would produce a number that means nothing"
        );
    }

    #[test]
    fn vectors_round_trip_through_storage() {
        let original = Embedding::new(vec![1.0, -2.0, 3.5, 0.25]);
        let restored = Embedding::from_bytes(&original.to_bytes()).unwrap();

        assert_eq!(restored, original);
        assert!((restored.similarity(&original) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_truncated_row_is_refused_rather_than_misread() {
        assert!(Embedding::from_bytes(&[0, 1, 2]).is_none());
        assert!(
            Embedding::from_bytes(&[]).is_some(),
            "empty is a valid zero-dimension vector"
        );
    }

    #[test]
    fn a_row_holding_nonsense_is_refused() {
        let bytes = f32::NAN.to_le_bytes().to_vec();
        assert!(Embedding::from_bytes(&bytes).is_none());
    }

    #[test]
    fn fidelity_says_what_a_number_is_worth() {
        assert_eq!(Fidelity::Lexical.as_str(), "lexical");
        assert!(Fidelity::Lexical.caveat().contains("not shared meaning"));
        assert!(Fidelity::Semantic.caveat().contains("model"));
    }
}
