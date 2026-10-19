use super::*;
use ratatui::crossterm::event::KeyCode;

fn kept() -> Vec<(String, String)> {
    let kept = [
        (
            "Qwen/Qwen2.5-3B-Instruct-GGUF",
            "qwen2.5-3b-instruct-q4_k_m.gguf",
        ),
        ("someone/Other-GGUF", "Other-Q4_K_M.gguf"),
        (
            "sentence-transformers/all-MiniLM-L6-v2",
            "model.safetensors",
        ),
    ];
    (kept.iter())
        .map(|(r, f)| (r.to_string(), f.to_string()))
        .collect()
}

#[test]
fn suggested_models_come_first_then_those_kept() {
    let entries = entries(&kept(), &Weights::default());
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "qwen2.5-3b",
            "qwen2.5-1.5b",
            "qwen3-4b",
            "llama3.2-3b",
            "qwen2.5-7b",
            "Other-Q4_K_M"
        ]
    );
    assert!(entries[0].kept && !entries[1].kept && entries[5].kept);
}

#[test]
fn a_model_given_is_listed_too() {
    let given = Weights::File("/models/mine.gguf".into());
    let entries = entries(&[], &given);
    let last = entries.last().unwrap();
    assert_eq!(
        (last.name.as_str(), &last.weights, last.kept),
        ("mine.gguf", &given, true)
    );
}

fn type_in(list: &mut List, text: &str) {
    for c in text.chars() {
        list.menu.key(KeyEvent::from(KeyCode::Char(c)), list.len());
    }
    list.refresh();
}

#[test]
fn typing_narrows_the_list_or_names_a_repo_of_the_hubs() {
    let mut list = List::new(entries(&kept(), &Weights::default()), Weights::default());
    assert_eq!(list.picked().unwrap().name, "qwen2.5-3b", "the one in use");
    type_in(&mut list, "llama");
    assert_eq!(list.picked().unwrap().name, "llama3.2-3b");
    list.menu.query.clear();
    type_in(&mut list, "bartowski/gemma-GGUF");
    let typed = list.picked().unwrap();
    assert_eq!(typed.about, "from Hugging Face");
    assert_eq!(
        typed.weights,
        Weights::Hub {
            repo: "bartowski/gemma-GGUF".into(),
            file: None,
        }
    );
    list.menu.query.clear();
    type_in(
        &mut list,
        "Qwen/Qwen2.5-3B-Instruct-GGUF/qwen2.5-3b-instruct-q4_k_m.gguf",
    );
    assert!(list.typed.is_none(), "listed already");
}

#[test]
fn enter_picks_the_model_and_esc_cancels() {
    let mut models = Models {
        list: Some(List::new(
            entries(&[], &Weights::default()),
            Weights::default(),
        )),
    };
    assert_eq!(models.key(KeyEvent::from(KeyCode::Down)), None);
    assert_eq!(
        models.key(KeyEvent::from(KeyCode::Enter)),
        Some(Weights::from(&SUGGESTED[1]))
    );
    assert!(!models.is_open());
    models.open(&Weights::default());
    assert_eq!(models.key(KeyEvent::from(KeyCode::Esc)), None);
    assert!(!models.is_open());
}
