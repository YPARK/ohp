//! A language model run in ohp, with candle, from a GGUF file of its
//! quantised weights, which carries its tokenizer too: Qwen2.5's, Qwen3's,
//! or Llama 3's, or another model of theirs. It runs on the GPU, through
//! Metal on macOS or CUDA where ohp is built with its `cuda` feature, and
//! on the CPU where there is none. A model the GPU cannot load or run, as
//! when its memory runs out, is loaded again on the CPU.
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
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
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
    /// The weights of the model `content` heads, read from `file`, onto
    /// `device`.
    fn from_gguf(
        content: gguf_file::Content,
        file: &mut File,
        device: &Device,
    ) -> candle_core::Result<Weights> {
        let arch = content.metadata.get("general.architecture");
        Ok(
            match arch.and_then(|v| v.to_string().ok()).map(String::as_str) {
                Some("qwen2") => Weights::Qwen2(quantized_qwen2::ModelWeights::from_gguf(
                    content, file, device,
                )?),
                Some("qwen3") => Weights::Qwen3(quantized_qwen3::ModelWeights::from_gguf(
                    content, file, device,
                )?),
                _ => Weights::Llama(quantized_llama::ModelWeights::from_gguf(
                    content, file, device,
                )?),
            },
        )
    }

    /// The weights in the GGUF file at `path`, onto `device`.
    fn read(path: &Path, device: &Device) -> Result<Weights, String> {
        let at = path.display();
        let mut file = File::open(path).map_err(|e| format!("cannot open {at}: {e}"))?;
        (gguf_file::Content::read(&mut file))
            .and_then(|content| Weights::from_gguf(content, &mut file, device))
            .map_err(|e| format!("cannot load {at}: {e}"))
    }

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
    /// The GGUF file, to load again on the CPU.
    path: PathBuf,
}

/// What `e` says, as a model's troubles are told.
pub fn text(e: impl Display) -> String {
    e.to_string()
}

impl Llm {
    /// The model in the GGUF file at `path`, on the GPU if it can be had
    /// and the model fits there, else on the CPU.
    pub fn load(path: &Path) -> Result<Llm, String> {
        let at = path.display();
        let why = |e: candle_core::Error| format!("cannot load {at}: {e}");
        let mut file = File::open(path).map_err(|e| format!("cannot open {at}: {e}"))?;
        let content = gguf_file::Content::read(&mut file).map_err(why)?;
        let meta = |key: &str| content.metadata.get(key);
        let string = |key: &str| meta(key).and_then(|v| v.to_string().ok()).cloned();
        let arch = string("general.architecture").unwrap_or_default();
        let template = string("tokenizer.chat_template").unwrap_or_default();
        let mut context = meta(&format!("{arch}.context_length"))
            .and_then(|v| v.to_u32().ok())
            .map_or(4096, |n| n as usize);
        // candle reads a Llama only as far as this, whatever its file says.
        if arch == "llama" {
            context = context.min(quantized_llama::MAX_SEQ_LEN);
        }
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
        let ends: Vec<u32> = (chat.ends().iter())
            .filter_map(|t| tokenizer.token_to_id(t))
            .collect();
        let (weights, device) =
            on_gpu_or_cpu(|device| Ok((Weights::read(path, &device)?, device)))?;
        Ok(Llm {
            weights,
            chat,
            tokenizer,
            device,
            ends,
            context,
            path: path.into(),
        })
    }

    /// The answer to `user`, told `system` first, of at most `most` tokens.
    /// `each` is told how far it has come as it reads the prompt and after
    /// each token of the answer, and ends it there by returning false.
    /// Where CUDA's GPU fails, as when its memory runs out, the model is
    /// loaded on the CPU, there to stay, and asked again; not Metal's,
    /// whose memory is the CPU's.
    pub fn answer(
        &mut self,
        system: &str,
        user: &str,
        most: usize,
        mut each: impl FnMut(Step) -> bool,
    ) -> Result<String, String> {
        let prompt = self.chat.prompt(system, user);
        let encoded = self.tokenizer.encode(prompt, false).map_err(text)?;
        let tokens = encoded.get_ids();
        if tokens.len() + most > self.context {
            return Err(format!("{} tokens are too many to read", tokens.len()));
        }
        match self.ask(tokens, most, &mut each) {
            Ok(answer) => Ok(answer),
            Err(Fault::Device(on_gpu)) if self.device.is_cuda() => {
                let failed = |on_cpu| format!("{on_gpu}; on the CPU: {on_cpu}");
                self.weights = Weights::read(&self.path, &Device::Cpu).map_err(failed)?;
                self.device = Device::Cpu;
                self.ask(tokens, most, each).map_err(|e| failed(e.into()))
            }
            Err(fault) => Err(fault.into()),
        }
    }

    /// The answer after `prompt`, which fits.
    fn ask(
        &mut self,
        prompt: &[u32],
        most: usize,
        mut each: impl FnMut(Step) -> bool,
    ) -> Result<String, Fault> {
        let mut tokens = prompt.to_vec();
        self.weights.forget();
        let mut logits = None;
        for (i, chunk) in tokens.chunks(CHUNK).enumerate() {
            if !each(Step::Reading(i * CHUNK, tokens.len())) {
                return Ok(String::new());
            }
            let input = self.input(chunk)?;
            logits = Some(self.weights.forward(&input, i * CHUNK)?);
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
                .and_then(|l| sampler.sample(&l))?;
            if self.ends.contains(&next) {
                break;
            }
            tokens.push(next);
            let said = (self.tokenizer)
                .decode(&tokens[start..], true)
                .map_err(|e| Fault::Words(text(e)))?;
            answer = unthought(&said).into();
            if !each(Step::Writing(&answer, tokens.len() - start)) {
                break;
            }
            let input = self.input(&[next])?;
            logits = self.weights.forward(&input, tokens.len() - 1)?;
        }
        Ok(answer)
    }

    /// `tokens` as the model reads them, a batch of one.
    fn input(&self, tokens: &[u32]) -> candle_core::Result<Tensor> {
        Tensor::new(tokens, &self.device)?.unsqueeze(0)
    }
}

/// Why an answer was not had: the model's work on its device failed, or
/// the tokenizer did.
enum Fault {
    Device(candle_core::Error),
    Words(String),
}

impl From<candle_core::Error> for Fault {
    fn from(e: candle_core::Error) -> Self {
        Fault::Device(e)
    }
}

impl From<Fault> for String {
    fn from(fault: Fault) -> Self {
        match fault {
            Fault::Device(e) => text(e),
            Fault::Words(why) => why,
        }
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

/// The GPU, where one can be had: through Metal on macOS, or CUDA where
/// ohp is built with its `cuda` feature. It is looked for once, and the
/// models share it.
fn gpu() -> Option<Device> {
    static GPU: OnceLock<Option<Device>> = OnceLock::new();
    let gpu = GPU.get_or_init(|| {
        #[cfg(target_os = "macos")]
        if let Ok(metal) = Device::new_metal(0) {
            return Some(metal);
        }
        #[cfg(feature = "cuda")]
        if let Ok(cuda) = Device::new_cuda(0) {
            return Some(cuda);
        }
        None
    });
    gpu.clone()
}

/// What `load` makes on the GPU, where there is one and it can, as when
/// its memory does not run out; else on the CPU.
pub fn on_gpu_or_cpu<T>(mut load: impl FnMut(Device) -> Result<T, String>) -> Result<T, String> {
    if let Some(Ok(made)) = gpu().map(&mut load) {
        return Ok(made);
    }
    load(Device::Cpu)
}

#[cfg(test)]
#[path = "tests/llm.rs"]
mod tests;
