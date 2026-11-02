use super::*;

#[test]
fn what_is_said_is_heard_as_said() {
    for said in [
        Said::Deck("/tmp/a b/deck.pdf".into()),
        Said::Page(12),
        Said::Black(true),
        Said::Black(false),
        Said::Key(Turn::Next),
        Said::Key(Turn::Prev),
        Said::Key(Turn::Black),
        Said::Bye,
    ] {
        assert_eq!(Said::parse(&said.line()), Some(said));
    }
    assert_eq!(Said::parse("page x"), None);
    assert_eq!(Said::parse("hello"), None);
}

#[test]
fn the_audience_joins_and_both_ends_hear_each_other() {
    let mut stage = Stage::new().unwrap();
    stage.write(b"%PDF-1.4\n%%EOF\n").unwrap();
    let socket = stage.socket();
    let pdf = stage.pdf();
    let joining = std::thread::spawn(move || join(&socket));
    let until = Instant::now() + Duration::from_secs(5);
    while !stage.arrived() {
        assert!(Instant::now() < until, "no audience");
        std::thread::sleep(Duration::from_millis(10));
    }
    let peer = stage.peer.as_mut().unwrap();
    assert!(peer.say(&Said::Page(3)));
    assert!(peer.say(&Said::Deck(pdf.clone())));
    let mut audience = joining.join().unwrap().unwrap();
    assert_eq!((audience.pdf.clone(), audience.page), (pdf, 3));
    assert!(audience.peer.say(&Said::Key(Turn::Next)));
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let heard = stage.peer.as_ref().unwrap().heard();
        if heard.contains(&Said::Key(Turn::Next)) {
            break;
        }
        assert!(Instant::now() < until, "nothing heard");
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(stage);
    let until = Instant::now() + Duration::from_secs(5);
    while !audience.peer.heard().contains(&Said::Bye) {
        assert!(Instant::now() < until, "no bye");
        std::thread::sleep(Duration::from_millis(10));
    }
}
