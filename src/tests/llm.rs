use super::*;
use std::time::Instant;

#[test]
fn each_model_is_asked_in_its_own_chat_format() {
    let qwen = Chat::ChatMl { think: false }.prompt("Be brief.", "Sum up.");
    assert_eq!(
        qwen,
        "<|im_start|>system\nBe brief.<|im_end|>\n<|im_start|>user\nSum up.<|im_end|>\n<|im_start|>assistant\n"
    );
    let thinking = Chat::ChatMl { think: true }.prompt("Be brief.", "Sum up.");
    assert!(thinking.ends_with("<|im_start|>assistant\n<think>\n\n</think>\n\n"));
    let llama = Chat::Llama3.prompt("Be brief.", "Sum up.");
    assert!(llama.starts_with(
        "<|begin_of_text|><|start_header_id|>system<|end_header_id|>\n\nBe brief.<|eot_id|>"
    ));
    assert!(llama.ends_with("<|start_header_id|>assistant<|end_header_id|>\n\n"));
    assert!(Chat::Llama3.ends().contains(&"<|eot_id|>"));
}

#[test]
fn a_thought_written_though_not_asked_for_is_left_out() {
    assert_eq!(
        unthought("<think>hmm</think>\n\nSUMMARY: x"),
        "\n\nSUMMARY: x"
    );
    assert_eq!(unthought("<think>still thinking"), "");
    assert_eq!(unthought("SUMMARY: x"), "SUMMARY: x");
}

/// A model downloaded, as `OHP_TEST_MODEL` names it: too large to be
/// fetched for a test, so these run only when asked for, with `--ignored`.
fn model() -> Llm {
    let path = std::env::var_os("OHP_TEST_MODEL").expect("OHP_TEST_MODEL names a GGUF file");
    Llm::load(Path::new(&path)).expect("model loads")
}

/// How many tokens of the answer `step` says are written, if it is one.
fn written(step: Step) -> Option<usize> {
    match step {
        Step::Writing(_, tokens) => Some(tokens),
        Step::Reading(..) => None,
    }
}

#[test]
#[ignore = "needs a model downloaded, named by OHP_TEST_MODEL"]
fn answers_and_streams() {
    let started = Instant::now();
    let mut llm = model();
    eprintln!("loaded in {:?}", started.elapsed());
    let (mut read, mut seen) = (0, 0);
    let started = Instant::now();
    let answer = llm
        .answer(
            "You answer in one short sentence.",
            "[1] Gene expression differs between cell types.\n\
             [2] We measured it in ten thousand single cells.\n\
             [3] Cell types were told apart by their expression alone.\n\n\
             Sum up these sentences, then give the numbers of the most important ones.\n\
             Answer in this form:\nSUMMARY: <summary>\nKEY: <numbers>",
            120,
            |step| {
                match written(step) {
                    Some(tokens) => seen = tokens,
                    None => read += 1,
                }
                true
            },
        )
        .expect("answers");
    let took = started.elapsed();
    eprintln!("{answer}\n{seen} tokens in {took:?}");
    assert!(read > 0 && seen > 0);
    assert!(answer.to_uppercase().contains("SUMMARY"), "{answer}");
}

#[test]
#[ignore = "needs a model downloaded, named by OHP_TEST_MODEL"]
fn reads_a_long_prompt_in_good_time() {
    let mut llm = model();
    llm.answer("", "Say yes.", 2, |_| true).expect("warms up");
    let page = "Cells were sorted by their expression of marker genes. ".repeat(160);
    let started = Instant::now();
    let mut steps = Vec::new();
    llm.answer("You are helpful.", &page, 1, |step| {
        if let Step::Reading(done, of) = step {
            steps.push((done, of));
        }
        true
    })
    .expect("answers");
    eprintln!("read about 1800 tokens in {:?}", started.elapsed());
    let started = Instant::now();
    let mut seen = 0;
    llm.answer(
        "You are helpful.",
        "Write a long story about cells.",
        200,
        |step| {
            seen = written(step).unwrap_or(seen);
            true
        },
    )
    .expect("answers");
    let secs = started.elapsed().as_secs_f32();
    eprintln!(
        "wrote {seen} tokens in {secs:.2}s: {:.1} a second",
        seen as f32 / secs
    );
    assert!(steps.len() > 10, "{steps:?}");
    assert!(steps.windows(2).all(|w| w[0].0 < w[1].0));
}

#[test]
#[ignore = "needs a model downloaded, named by OHP_TEST_MODEL"]
fn stops_when_told() {
    let mut llm = model();
    let mut seen = 0;
    llm.answer(
        "You are helpful.",
        "Count from one to fifty.",
        200,
        |step| {
            seen = written(step).unwrap_or(seen);
            seen < 3
        },
    )
    .expect("answers");
    assert_eq!(seen, 3);
}
