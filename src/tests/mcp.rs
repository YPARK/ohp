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
    let mut want = vec![
        "status", "open", "goto", "slides", "text", "render", "notes",
    ];
    if cfg!(feature = "sign") {
        want.extend(["fields", "annotate"]);
    }
    want.extend(["printers", "print"]);
    assert_eq!(names, want);
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

/// A one-page PDF with a text field `Name` and a signature field `Sig`,
/// at these boxes, in PDF points, y up.
#[cfg(feature = "sign")]
fn form(dir: &std::path::Path) -> PathBuf {
    use lopdf::{Document, Object, dictionary};
    let mut doc = Document::load_mem(&fixture::pdf(1)).unwrap();
    let field = |doc: &mut Document, kind: &str, name: &str, rect: [f32; 4]| {
        doc.add_object(dictionary! {
            "Type" => "Annot",
            "Subtype" => "Widget",
            "FT" => kind,
            "T" => Object::string_literal(name),
            "Rect" => rect.iter().map(|&n| Object::Real(n)).collect::<Vec<_>>(),
        })
    };
    let name = field(&mut doc, "Tx", "Name", [50., 150., 250., 170.]);
    let sig = field(&mut doc, "Sig", "Sig", [50., 50., 250., 90.]);
    let page = *doc.get_pages().get(&1).unwrap();
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set(
            "Annots",
            vec![Object::Reference(name), Object::Reference(sig)],
        );
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    fixture::write_pdf(dir, &out)
}

#[cfg(feature = "sign")]
#[test]
fn a_pdf_s_fields_are_listed_and_marks_put_in_a_copy() {
    let dir = tempfile::tempdir().unwrap();
    let pdf = form(dir.path());
    let before = std::fs::read(&pdf).unwrap();
    let path = pdf.to_str().unwrap();
    let mut server = Server::default();

    let fields = said(&call(&mut server, "fields", json!({"path": path})));
    let h = fixture::FRAME.1;
    assert!(
        fields.contains(&format!(
            "text field \"Name\": [50.0, {:.1}, 250.0, {:.1}]",
            h - 170.,
            h - 150.
        )),
        "{fields}"
    );
    assert!(fields.contains("signature field \"Sig\""), "{fields}");

    let put = call(
        &mut server,
        "annotate",
        json!({"path": path, "marks": [
            {"what": "text", "text": "Jane Roe", "field": "Name"},
            {"what": "check", "page": 1, "x": 300, "y": 40, "size": 2},
            {"what": "date", "page": 1, "x": 300, "y": 80},
        ]}),
    );
    assert_eq!(put["isError"], false, "{put}");
    let put = said(&put);
    assert!(put.contains("fitted to field \"Name\""), "{put}");
    assert!(
        put.contains("✓ on page 1, over [286.0, 26.0, 314.0, 54.0] pt at 200%"),
        "{put}"
    );
    let copy = dir.path().join("deck-signed.pdf");
    let signed = std::fs::read(&copy).unwrap();
    // The PDF is kept as it was, and the copy is it with more after.
    assert_eq!(std::fs::read(&pdf).unwrap(), before);
    assert!(signed.len() > before.len() && signed.starts_with(&before));
    let text = said(&call(
        &mut server,
        "text",
        json!({"path": copy.to_str().unwrap()}),
    ));
    assert!(text.contains("Jane Roe"), "{text}");

    // Called again, the marks are drawn afresh: what was put before is gone.
    let again = call(
        &mut server,
        "annotate",
        json!({"path": path, "marks": [{"what": "dot", "page": 1, "x": 20, "y": 20}], "name": "done"}),
    );
    assert_eq!(again["isError"], false, "{again}");
    let done = dir.path().join("done.pdf");
    let text = said(&call(
        &mut server,
        "text",
        json!({"path": done.to_str().unwrap()}),
    ));
    assert!(!text.contains("Jane Roe"), "{text}");
    let dot = json!([{"what": "dot", "page": 1, "x": 20, "y": 20}]);
    // Its own copy, as it left it, is written over.
    let redrawn = call(&mut server, "annotate", json!({"path": path, "marks": dot}));
    assert_eq!(redrawn["isError"], false, "{redrawn}");

    // A copy changed since, as by `A` on screen, or a file it did not make,
    // only if asked.
    std::fs::write(&copy, b"signed on screen").unwrap();
    let kept = call(&mut server, "annotate", json!({"path": path, "marks": dot}));
    assert_eq!(kept["isError"], true, "{kept}");
    assert!(said(&kept).contains("`replace`"), "{}", said(&kept));
    assert_eq!(std::fs::read(&copy).unwrap(), b"signed on screen");
    let other = dir.path().join("budget.pdf");
    std::fs::write(&other, b"mine").unwrap();
    let named = json!({"path": path, "marks": dot, "name": "budget"});
    assert_eq!(
        call(&mut server, "annotate", named.clone())["isError"],
        true
    );
    assert_eq!(std::fs::read(&other).unwrap(), b"mine");
    let mut replace = named;
    replace["replace"] = json!(true);
    assert_eq!(call(&mut server, "annotate", replace)["isError"], false);
    assert!(std::fs::read(&other).unwrap().starts_with(b"%PDF"));
}

#[cfg(feature = "sign")]
#[test]
fn a_signature_is_fitted_to_its_field_and_what_cannot_be_put_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let pdf = form(dir.path());
    let path = pdf.to_str().unwrap();
    let ink = dir.path().join("ink.png");
    let mut image = image::RgbaImage::from_pixel(300, 60, image::Rgba([255, 255, 255, 0]));
    for x in 10..290 {
        for y in 20..40 {
            image.put_pixel(x, y, image::Rgba([0, 0, 120, 255]));
        }
    }
    image.save(&ink).unwrap();
    let mut server = Server::default();
    let h = fixture::FRAME.1;
    // Clicked in the field, as on screen.
    let signed = call(
        &mut server,
        "annotate",
        json!({"path": path, "signature": ink.to_str().unwrap(), "marks": [
            {"what": "signature", "page": 1, "x": 100, "y": h - 70.},
        ]}),
    );
    assert_eq!(signed["isError"], false, "{signed}");
    assert!(
        said(&signed).contains("fitted to the form field there"),
        "{}",
        said(&signed)
    );

    let refused = |server: &mut Server, args: Value, why: &str| {
        let result = call(server, "annotate", args);
        assert_eq!(result["isError"], true, "{result}");
        assert!(said(&result).contains(why), "{}", said(&result));
    };
    refused(&mut server, json!({"path": path, "marks": []}), "`marks`");
    refused(
        &mut server,
        json!({"path": path, "marks": [{"what": "text", "text": "名前", "page": 1, "x": 1, "y": 1}]}),
        "Helvetica",
    );
    refused(
        &mut server,
        json!({"path": path, "marks": [{"what": "text", "text": "x", "field": "Nope"}]}),
        "no field",
    );
    refused(
        &mut server,
        json!({"path": path, "marks": [{"what": "check", "page": 1, "x": 1}]}),
        "`y`",
    );
    refused(
        &mut server,
        json!({"path": path, "marks": [{"what": "check", "page": 1, "x": 1, "y": 1}], "name": "deck"}),
        "the PDF itself",
    );
    if std::env::var_os("OHP_SIGNATURE").is_none() {
        refused(
            &mut server,
            json!({"path": path, "marks": [{"what": "signature", "page": 1, "x": 1, "y": 1}]}),
            "no signature is kept",
        );
    }
    for x in [1e39, -5.] {
        refused(
            &mut server,
            json!({"path": path, "marks": [{"what": "check", "page": 1, "x": x, "y": 1}]}),
            "off page 1",
        );
    }
    let md = fixture::write_text(dir.path(), "talk.md", "# Hi\n");
    refused(
        &mut server,
        json!({"path": md.to_str().unwrap(), "marks": [{"what": "dot", "x": 1, "y": 1}]}),
        "only a PDF",
    );
}

#[test]
fn printing_what_cannot_be_printed_is_refused_before_it_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    let pdf = fixture::write(dir.path(), 3);
    let path = pdf.to_str().unwrap();
    let mut server = Server::default();
    for (args, why) in [
        (
            json!({"path": path, "sides": "both"}),
            "`sides` is one of printer, one, two",
        ),
        (json!({"path": path, "pages": "7"}), "no page 7"),
        (json!({"path": path, "copies": 0}), "1 to 99"),
        (json!({"path": path, "pages": "this"}), "no page `this`"),
    ] {
        let result = call(&mut server, "print", args);
        assert_eq!(result["isError"], true, "{result}");
        assert!(said(&result).contains(why), "{}", said(&result));
    }
}
