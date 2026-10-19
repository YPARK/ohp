//! Sentences as vectors of what they mean, by a small sentence embedding
//! model, sentence-transformers' all-MiniLM-L6-v2, run in ohp with candle:
//! its BERT encoder's last states, averaged over each sentence's tokens and
//! scaled to length one, so the dot product of two is their cosine. It runs
//! where the language model does, on the GPU on macOS, before it.

use crate::llm::{self, text};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use std::fmt::Display;
use std::path::Path;
use tokenizers::{PaddingParams, Tokenizer, TruncationParams};

/// The model, on Hugging Face's Hub, and the files of it used.
pub const REPO: &str = "sentence-transformers/all-MiniLM-L6-v2";
pub const FILES: [&str; 3] = ["config.json", "tokenizer.json", "model.safetensors"];
/// Sentences embedded at once.
const BATCH: usize = 32;
/// Most tokens of a sentence embedded: past these, it is cut short, as the
/// model was trained.
const MOST_TOKENS: usize = 256;

pub struct Embedder {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
}

/// Why the model cannot be loaded, as `e` says.
fn why(e: impl Display) -> String {
    format!("cannot load {REPO}: {e}")
}

impl Embedder {
    /// The model in `dir`, which has its `FILES`.
    pub fn load(dir: &Path) -> Result<Embedder, String> {
        let config = std::fs::read_to_string(dir.join(FILES[0])).map_err(why)?;
        let config: Config = serde_json::from_str(&config).map_err(why)?;
        let mut tokenizer = Tokenizer::from_file(dir.join(FILES[1])).map_err(why)?;
        tokenizer.with_padding(Some(PaddingParams::default()));
        let truncation = TruncationParams {
            max_length: MOST_TOKENS,
            ..TruncationParams::default()
        };
        tokenizer.with_truncation(Some(truncation)).map_err(why)?;
        let device = llm::device();
        let weights = [dir.join(FILES[2])];
        // SAFETY: the weights are a file of the cache's, not written while
        // ohp runs.
        let vb = unsafe { VarBuilder::from_mmaped_safetensors(&weights, DType::F32, &device) }
            .map_err(why)?;
        let model = BertModel::load(vb, &config).map_err(why)?;
        Ok(Embedder {
            model,
            tokenizer,
            device,
        })
    }

    /// `sentences` embedded, a batch at a time, `each` told how many are
    /// after each; `None` once `each` returns false. Sentences of a length
    /// are embedded together: a batch is as long as its longest.
    pub fn embed(
        &self,
        sentences: &[&str],
        mut each: impl FnMut(usize) -> bool,
    ) -> Result<Option<Vec<Vec<f32>>>, String> {
        let mut order: Vec<usize> = (0..sentences.len()).collect();
        order.sort_by_key(|&i| sentences[i].len());
        let mut vectors = vec![Vec::new(); sentences.len()];
        for (done, batch) in order.chunks(BATCH).enumerate() {
            if !each(done * BATCH) {
                return Ok(None);
            }
            let said: Vec<&str> = batch.iter().map(|&i| sentences[i]).collect();
            for (&i, vector) in batch.iter().zip(self.batch(&said)?) {
                vectors[i] = vector;
            }
        }
        Ok(Some(vectors))
    }

    fn batch(&self, sentences: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        let encoded = (self.tokenizer)
            .encode_batch(sentences.to_vec(), true)
            .map_err(text)?;
        let rows = |of: &dyn Fn(&tokenizers::Encoding) -> &[u32]| {
            let rows: Vec<Tensor> = (encoded.iter())
                .map(|e| Tensor::new(of(e), &self.device))
                .collect::<candle_core::Result<_>>()?;
            Tensor::stack(&rows, 0)
        };
        let ids = rows(&|e| e.get_ids()).map_err(text)?;
        let mask = rows(&|e| e.get_attention_mask()).map_err(text)?;
        let types = ids.zeros_like().map_err(text)?;
        let states = (self.model)
            .forward(&ids, &types, Some(&mask))
            .map_err(text)?;
        // The states of the tokens, not the padding, averaged.
        let mean = (|| {
            let mask = mask.to_dtype(DType::F32)?.unsqueeze(2)?;
            let sum = states.broadcast_mul(&mask)?.sum(1)?;
            let mean = sum.broadcast_div(&mask.sum(1)?)?;
            let norm = mean.sqr()?.sum_keepdim(1)?.sqrt()?;
            mean.broadcast_div(&norm)?.to_vec2::<f32>()
        })();
        mean.map_err(text)
    }
}

#[cfg(test)]
#[path = "tests/embed.rs"]
mod tests;
