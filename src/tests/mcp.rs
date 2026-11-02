use super::*;
use crate::fixture;

fn call(server: &mut Server, name: &str, args: Value) -> Value {
    let message = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": name, "arguments": args}});
    let answer = server.handle(&message).unwrap();
    assert_eq!(answer["id"], 7);
    answer["result"].clone()
}

fn said(result: &Value) -> String {
    result["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn it_says_what_it_is_and_lists_its_tools() {
    let mut server = Server::default();
    let init = server
        .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}}))
        .unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(init["result"]["serverInfo"]["name"], "ohp");
    assert!(init["result"]["capabilities"]["tools"].is_object());
    // A notification has no answer.
    assert!(
        server
            .handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .is_none()
    );
    let list = server
        .handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))
        .unwrap();
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "status", "open", "goto", "slides", "text", "render", "notes"
        ]
    );
    for tool in list["result"]["tools"].as_array().unwrap() {
        assert_eq!(tool["inputSchema"]["type"], "object");
    }
    let unknown = server
        .handle(&json!({"jsonrpc": "2.0", "id": 3, "method": "nope"}))
        .unwrap();
    assert_eq!(unknown["error"]["code"], -32601);
}

#[test]
fn a_decks_pages_are_listed_read_and_rendered() {
    let dir = tempfile::tempdir().unwrap();
    let pdf = fixture::write_pdf(dir.path(), &fixture::saying(&["Results", "Methods"]));
    let path = pdf.to_str().unwrap();
    let mut server = Server::default();

    let slides = call(&mut server, "slides", json!({"path": path}));
    let listed = said(&slides);
    assert!(listed.contains("2 pages"), "{listed}");
    assert!(
        listed.contains("1. Results") && listed.contains("2. Methods"),
        "{listed}"
    );

    let text = call(&mut server, "text", json!({"path": path, "page": 2}));
    assert_eq!(said(&text).trim(), "Methods");

    let image = call(
        &mut server,
        "render",
        json!({"path": path, "page": 1, "width": 300}),
    );
    assert_eq!(image["isError"], false);
    assert_eq!(image["content"][0]["mimeType"], "image/png");
    let png = base64::engine::general_purpose::STANDARD
        .decode(image["content"][0]["data"].as_str().unwrap())
        .unwrap();
    assert_eq!(&png[1..4], b"PNG");
    let decoded = image::load_from_memory(&png).unwrap();
    assert_eq!(decoded.width(), 300);

    let past = call(&mut server, "render", json!({"path": path, "page": 9}));
    assert_eq!(past["isError"], true);
    assert!(said(&past).contains("no page 9"));
}

#[test]
fn notes_are_written_beside_the_deck_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let pdf = fixture::write(dir.path(), 3);
    let path = pdf.to_str().unwrap();
    let mut server = Server::default();
    let none = call(&mut server, "notes", json!({"path": path, "page": 2}));
    assert!(said(&none).contains("no notes"));
    let wrote = call(
        &mut server,
        "notes",
        json!({"path": path, "page": 2, "text": "Say **why**."}),
    );
    assert_eq!(wrote["isError"], false, "{wrote}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("deck.md")).unwrap(),
        "# 2\nSay **why**.\n"
    );
    let read = call(&mut server, "notes", json!({"path": path, "page": 2}));
    assert_eq!(said(&read), "Say **why**.");
    let all = call(&mut server, "notes", json!({"path": path}));
    assert_eq!(said(&all), "# 2\nSay **why**.\n");
    let slides = call(&mut server, "slides", json!({"path": path}));
    assert!(said(&slides).contains("2. "), "{}", said(&slides));
    let unpaged = call(&mut server, "notes", json!({"path": path, "text": "x"}));
    assert_eq!(unpaged["isError"], true);
}

#[test]
fn a_file_that_is_not_there_is_said_to_be_missing() {
    let mut server = Server::default();
    let result = call(&mut server, "slides", json!({"path": "/no/such/talk.pdf"}));
    assert_eq!(result["isError"], true);
    assert!(said(&result).contains("there is no"));
}

#[test]
fn the_running_ohps_status_is_heard_as_said() {
    let status = Status {
        files: vec!["/a/talk.pdf".into(), "/b/more.md".into()],
        page: 2,
        pages: 9,
        view: "grid".into(),
        presenting: true,
        paper: Some("a4".into()),
        knit: true,
        screen: false,
    };
    assert_eq!(Status::parse(&status.lines()), status);
}
#[test]
fn a_page_that_is_not_a_whole_number_is_refused_not_guessed() {
    let dir = tempfile::tempdir().unwrap();
    let pdf = fixture::write(dir.path(), 3);
    let path = pdf.to_str().unwrap();
    let mut server = Server::default();
    for page in [json!("3"), json!(3.5), json!(-1)] {
        let wrote = call(
            &mut server,
            "notes",
            json!({"path": path, "page": page, "text": "x"}),
        );
        assert_eq!(wrote["isError"], true, "{page}");
        assert!(said(&wrote).contains("whole number"), "{}", said(&wrote));
        let seen = call(&mut server, "render", json!({"path": path, "page": page}));
        assert_eq!(seen["isError"], true, "{page}");
    }
    assert!(
        !dir.path().join("deck.md").exists(),
        "no notes were written"
    );
}

#[test]
fn r_chunks_are_not_run_reading_a_file_named() {
    let dir = tempfile::tempdir().unwrap();
    let ran = dir.path().join("ran");
    let rmd = fixture::write_text(
        dir.path(),
        "talk.Rmd",
        &format!(
            "# Hi\n\n```{{r}}\nfile.create({:?})\n```\n",
            ran.to_str().unwrap()
        ),
    );
    let mut server = Server::default();
    let text = call(&mut server, "text", json!({"path": rmd.to_str().unwrap()}));
    assert_eq!(text["isError"], false, "{text}");
    assert!(!ran.exists(), "the chunk ran");
}
