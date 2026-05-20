use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const DEFAULT_MODEL: &str = "nomic-embed-text";
pub const EMBED_DIM: usize = 768; // nomic-embed-text output dimension

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    prompt: &'a str,
}

/// Handles both legacy `{"embedding": [...]}` and newer `{"embeddings": [[...]]}` formats.
#[derive(Deserialize)]
struct EmbedResponse {
    embedding: Option<Vec<f32>>,
    embeddings: Option<Vec<Vec<f32>>>,
}

pub struct Embedder {
    client: Client,
    base_url: String,
    model: String,
}

impl Embedder {
    pub fn new(ollama_url: &str) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("reqwest client"),
            base_url: ollama_url.trim_end_matches('/').to_string(),
            model: DEFAULT_MODEL.to_string(),
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Returns true if Ollama is reachable and the embedding model is available.
    pub async fn is_available(&self) -> bool {
        self.client
            .get(format!("{}/api/tags", self.base_url))
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    /// Compute an embedding for the given text.
    /// Returns a zero vector if Ollama is unavailable — callers should check `is_available` first.
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let url = format!("{}/api/embeddings", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&EmbedRequest {
                model: &self.model,
                prompt: text,
            })
            .send()
            .await
            .context("Failed to reach Ollama — is it running?")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if body.contains("model not found") || body.contains("pull") {
                anyhow::bail!(
                    "Model '{}' not found. Pull it with: ollama pull {}",
                    self.model, self.model
                );
            }
            anyhow::bail!("Ollama error {status}: {}", &body[..body.len().min(200)]);
        }

        let data: EmbedResponse = resp.json().await.context("Invalid embedding response")?;

        // Handle both response shapes
        let embedding = data
            .embedding
            .or_else(|| data.embeddings.and_then(|mut v| v.pop().map(Some)).flatten())
            .context("No embedding in response")?;

        if embedding.is_empty() {
            anyhow::bail!("Ollama returned empty embedding");
        }

        Ok(embedding)
    }

    /// Embed multiple texts with a concurrency cap of 4 simultaneous requests.
    pub async fn embed_batch(&self, texts: &[String]) -> Vec<Result<Vec<f32>>> {
        use futures::stream::{self, StreamExt};
        stream::iter(texts)
            .map(|t| self.embed(t))
            .buffered(4)
            .collect()
            .await
    }

    /// Zero vector — used as a placeholder when embedding is unavailable.
    pub fn zero_embedding() -> Vec<f32> {
        vec![0.0f32; EMBED_DIM]
    }
}

/// Cosine similarity between two vectors. Returns 0.0 if either is all-zero.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a < f32::EPSILON || norm_b < f32::EPSILON {
        0.0
    } else {
        (dot / (norm_a * norm_b)).clamp(-1.0, 1.0)
    }
}

/// Serialize embedding to raw LE bytes for SQLite BLOB storage.
pub fn to_blob(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Deserialize embedding from raw LE bytes.
pub fn from_blob(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}
