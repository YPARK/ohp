//! A language model run in ohp, with candle, from a GGUF file of its
//! quantised weights, which carries its tokenizer too: Qwen2.5's, Qwen3's,
//! or Llama 3's, or another model of theirs. On macOS it runs on the GPU,
//! through Metal; elsewhere on the CPU.
//!
//! Each is asked as it was taught to be, in its own chat format: Qwen's
//! ChatML or Llama 3's headers. A Qwen3 that can think first is told not
//! to, with the empty thought its template gives for that.
//!
//! Answers are greedy, the likeliest token each step, so a page is summed
//! up the same way each time; a mild penalty on tokens the answer has just
//! said keeps a small model from going round in circles. The prompt is not
//! penalised: an answer quoting it is what is asked for.

use crate::summary::Step;
use candle_core::quantized::gguf_file;
use candle_core::quantized::tokenizer::TokenizerFromGguf;
use candle_core::{Device, Tensor};
use candle_transformers::generation::LogitsProcessor;
use candle_transformers::models::{quantized_llama, quantized_qwen2, quantized_qwen3};
use candle_transformers::utils::apply_repeat_penalty;
use std::fmt::Display;
use std::path::Path;
use tokenizers::Tokenizer;

/// Tokens of the prompt read at once: small enough to show how far the
/// reading is, and no slower than more.
const CHUNK: usize = 128;
/// How hard the answer's last tokens are penalised, and how many of them.
const REPEAT_PENALTY: f32 = 1.1;
const REPEAT_LAST: usize = 64;

enum Weights {
    Qwen2(quantized_qwen2::ModelWeights),
    Qwen3(quantized_qwen3::ModelWeights),
    Llama(quantized_llama::ModelWeights),
}

impl Weights {
    fn forward(&mut self, input: &Tensor, at: usize) -> candle_core::Result<Tensor> {
        match self {
            Weights::Qwen2(m) => m.forward(input, at),
            Weights::Qwen3(m) => m.forward(input, at),
            Weights::Llama(m) => m.forward(input, at),
        }
    }

    /// Forget the last prompt: Qwen3 does not when read from the start.
    fn forget(&mut self) {
        if let Weights::Qwen3(m) = self {
            m.clear_kv_cache();
        }
    }
}

/// How a model is asked.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Chat {
    /// Qwen's; `think` where it would think first unless given a thought.
    ChatMl {
        think: bool,
    },
    Llama3,
}

impl Chat {
    fn prompt(self, system: &str, user: &str) -> String {
        match self {
            Chat::ChatMl { think } => {
                let thought = if think { "<think>\n\n</think>\n\n" } else { "" };
                format!(
                    "<|im_start|>system\n{system}<|im_end|>\n\
                     <|im_start|>user\n{user}<|im_end|>\n\
                     <|im_start|>assistant\n{thought}"
                )
            }
            Chat::Llama3 => format!(
                "<|begin_of_text|><|start_header_id|>system<|end_header_id|>\n\n{system}<|eot_id|>\
                 <|start_header_id|>user<|end_header_id|>\n\n{user}<|eot_id|>\
                 <|start_header_id|>assistant<|end_header_id|>\n\n"
            ),
        }
    }

    /// Tokens that end an answer.
    fn ends(self) -> &'static [&'static str] {
        match self {
            Chat::ChatMl { .. } => &["<|im_end|>", "<|endoftext|>"],
            Chat::Llama3 => &["<|eot_id|>", "<|end_of_text|>", "<|eom_id|>"],
        }
    }
}

pub struct Llm {
    weights: Weights,
    chat: Chat,
    tokenizer: Tokenizer,
    device: Device,
    ends: Vec<u32>,
    /// Most tokens of prompt and answer together.
    context: usize,
}

/// What `e` says, as a model's troubles are told.
pub fn text(e: impl Display) -> String {
    e.to_string()
}

impl Llm {
    /// The model in the GGUF file at `path`.
    pub fn load(path: &Path) -> Result<Llm, String> {
        let at = path.display();
        let why = |e: candle_core::Error| format!("cannot load {at}: {e}");
        let mut file = std::fs::File::open(path).map_err(|e| format!("cannot open {at}: {e}"))?;
        let content = gguf_file::Content::read(&mut file).map_err(why)?;
        let meta = |key: &str| content.metadata.get(key);
        let string = |key: &str| meta(key).and_then(|v| v.to_string().ok()).cloned();
        let arch = string("general.architecture").unwrap_or_default();
        let template = string("tokenizer.chat_template").unwrap_or_default();
        let context = meta(&format!("{arch}.context_length"))
            .and_then(|v| v.to_u32().ok())
            .map_or(4096, |n| n as usize);
        let tokenizer = Tokenizer::from_gguf(&content)
            .map_err(|e| format!("cannot read {at}'s tokenizer: {e}"))?;
        let chat = match arch.as_str() {
            "qwen2" => Chat::ChatMl { think: false },
            "qwen3" => Chat::ChatMl {
                think: template.contains("enable_thinking"),
            },
            "llama" if tokenizer.token_to_id("<|start_header_id|>").is_some() => Chat::Llama3,
            _ => {
                return Err(format!(
                    "{at} is a model ohp cannot ask: {arch}; it asks Qwen2, Qwen3 and Llama 3"
                ));
            }
        };
        let device = device();
        let weights = match arch.as_str() {
            "qwen2" => Weights::Qwen2(
                quantized_qwen2::ModelWeights::from_gguf(content, &mut file, &device)
                    .map_err(why)?,
            ),
            "qwen3" => Weights::Qwen3(
                quantized_qwen3::ModelWeights::from_gguf(content, &mut file, &device)
                    .map_err(why)?,
            ),
            _ => Weights::Llama(
                quantized_llama::ModelWeights::from_gguf(content, &mut file, &device)
                    .map_err(why)?,
            ),
        };
        let ends = (chat.ends().iter())
            .filter_map(|t| tokenizer.token_to_id(t))
            .collect();
        Ok(Llm {
            weights,
            chat,
            tokenizer,
            device,
            ends,
            context,
        })
    }

    /// The answer to `user`, told `system` first, of at most `most` tokens.
    /// `each` is told how far it has come as it reads the prompt and after
    /// each token of the answer, and ends it there by returning false.
    pub fn answer(
        &mut self,
        system: &str,
        user: &str,
        most: usize,
        mut each: impl FnMut(Step) -> bool,
    ) -> Result<String, String> {
        let prompt = self.chat.prompt(system, user);
        let encoded = self.tokenizer.encode(prompt, false).map_err(text)?;
        let mut tokens = encoded.get_ids().to_vec();
        if tokens.len() + most > self.context {
            return Err(format!("{} tokens are too many to read", tokens.len()));
        }
        self.weights.forget();
        let mut logits = None;
        for (i, chunk) in tokens.chunks(CHUNK).enumerate() {
            if !each(Step::Reading(i * CHUNK, tokens.len())) {
                return Ok(String::new());
            }
            let input = self.input(chunk)?;
            logits = Some(self.weights.forward(&input, i * CHUNK).map_err(text)?);
        }
        let Some(mut logits) = logits else {
            return Ok(String::new());
        };
        let mut sampler = LogitsProcessor::new(0, None, None);
        let start = tokens.len();
        let mut answer = String::new();
        for _ in 0..most {
            let recent = &tokens[tokens.len().saturating_sub(REPEAT_LAST).max(start)..];
            let next = logits
                .squeeze(0)
                .and_then(|l| apply_repeat_penalty(&l, REPEAT_PENALTY, recent))
                .and_then(|l| sampler.sample(&l))
                .map_err(text)?;
            if self.ends.contains(&next) {
                break;
            }
            tokens.push(next);
            let said = (self.tokenizer)
                .decode(&tokens[start..], true)
                .map_err(text)?;
            answer = unthought(&said).into();
            if !each(Step::Writing(&answer, tokens.len() - start)) {
                break;
            }
            let input = self.input(&[next])?;
            logits = (self.weights)
                .forward(&input, tokens.len() - 1)
                .map_err(text)?;
        }
        Ok(answer)
    }

    /// `tokens` as the model reads them, a batch of one.
    fn input(&self, tokens: &[u32]) -> Result<Tensor, String> {
        (Tensor::new(tokens, &self.device))
            .and_then(|t| t.unsqueeze(0))
            .map_err(text)
    }
}

/// `said` without a thought before it, as a Qwen3 may write though told
/// not to.
fn unthought(said: &str) -> &str {
    match said.trim_start().strip_prefix("<think>") {
        Some(thinking) => thinking.split_once("</think>").map_or("", |(_, said)| said),
        None => said,
    }
}

/// The GPU, through Metal, on macOS where it can be had; else the CPU.
pub fn device() -> Device {
    #[cfg(target_os = "macos")]
    if let Ok(metal) = Device::new_metal(0) {
        return metal;
    }
    Device::Cpu
}

#[cfg(test)]
#[path = "tests/llm.rs"]
mod tests;
