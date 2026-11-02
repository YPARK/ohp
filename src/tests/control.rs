use super::*;

#[test]
fn a_request_is_heard_as_asked() {
    for request in [
        Request::Status,
        Request::Goto(4),
        Request::Open("/tmp/a talk.pdf".into(), 2),
    ] {
        assert_eq!(Request::parse(&request.line()), Some(request));
    }
    assert_eq!(Request::parse("goto 0"), None);
    assert_eq!(Request::parse("rm -rf"), None);
}

#[test]
fn the_running_ohp_answers_what_it_is_asked() {
    let dir = tempfile::tempdir().unwrap();
    let here = dir.path().join("ohp");
    assert_eq!(ask_in(&here, &Request::Status), None, "none is running");
    let listener = Listener::in_dir(&here).unwrap();
    let mode =
        std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&here).unwrap().permissions());
    assert_eq!(mode & 0o777, 0o700);
    let answering = std::thread::spawn(move || {
        let (request, reply) = listener.asked.recv().unwrap();
        assert_eq!(request, Request::Goto(2));
        reply.send(vec!["ok slide 3".into()]).unwrap();
        listener
    });
    assert_eq!(
        ask_in(&here, &Request::Goto(2)),
        Some(vec!["ok slide 3".into()])
    );
    drop(answering.join().unwrap());
    assert_eq!(
        ask_in(&here, &Request::Status),
        None,
        "its socket is gone with it"
    );
}
