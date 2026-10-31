use super::*;
use crate::fixture::fake_lp;

/// The pages `text` names, as CUPS is sent them.
fn said(text: &str, current: usize, total: usize) -> Result<Option<String>, String> {
    pages(text, current, total).map(|r| r.map(|r| r.to_string()))
}

/// The ranges `text` names, of `total` pages.
fn ranges(text: &str, total: usize) -> Ranges {
    pages(text, 0, total).unwrap().unwrap()
}
use std::os::unix::fs::PermissionsExt;

#[test]
fn pages_are_read_as_cups_takes_them() {
    assert_eq!(said("", 3, 10), Ok(None));
    assert_eq!(said(" all ", 3, 10), Ok(None));
    assert_eq!(said("this", 3, 10), Ok(Some("4".into())));
    assert_eq!(said(".", 0, 10), Ok(Some("1".into())));
    assert_eq!(said("3-7, 9", 0, 10), Ok(Some("3-7,9".into())));
    assert_eq!(said("8-", 0, 10), Ok(Some("8-10".into())));
    assert_eq!(said("-2", 0, 10), Ok(Some("1-2".into())));
    assert_eq!(said("5-5", 0, 10), Ok(Some("5".into())));
}

#[test]
fn pages_past_the_end_or_backwards_are_refused() {
    assert!(pages("0", 0, 10).is_err());
    assert!(pages("11", 0, 10).is_err());
    assert!(pages("7-3", 0, 10).is_err());
    assert!(pages("two", 0, 10).is_err());
    assert!(pages("1,,2", 0, 10).is_err());
}

#[test]
fn queues_are_listed_the_default_first_then_printers_found_only_on_the_network() {
    let queues = "printer Office is idle.  enabled since Wed\n\
                  \tDescription: Office laser\n\
                  printer HP_Laser disabled since Tue -\n\
                  \treason unknown\n\
                  system default destination: HP_Laser\n";
    let found = "Office\nHP_Laser\nHP_Laser_Bonjour\n";
    let all = destinations(queues, found);
    assert_eq!(all.names, ["HP_Laser", "Office", "HP_Laser_Bonjour"]);
    assert!(all.default);
    // With none named the default, the first is not called it.
    let none = "printer Office is idle.\nno system default destination\n";
    let all = destinations(none, "");
    assert_eq!(
        (all.names, all.default),
        (vec!["Office".to_string()], false)
    );
    assert!(destinations("", "").names.is_empty());
}

#[test]
fn lp_is_asked_for_what_is_picked() {
    let job = Job {
        printer: Some("Office".into()),
        copies: 2,
        pages: Some("3-7".into()),
        sides: Sides::Two,
        scale: Scale::Auto,
        wide: false,
        title: "talk.pdf".into(),
    };
    assert_eq!(
        args(&job, &LP),
        [
            "-d",
            "Office",
            "-n",
            "2",
            "-o",
            "page-ranges=3-7",
            "-o",
            "sides=two-sided-long-edge",
            "-t",
            "talk.pdf",
        ]
    );
    // The sides left to the printer, unless asked.
    assert_eq!(args(&Job::new("a.pdf"), &LP), ["-t", "a.pdf"]);
    let one = Job {
        sides: Sides::One,
        ..Job::new("a.pdf")
    };
    assert_eq!(args(&one, &LP), ["-o", "sides=one-sided", "-t", "a.pdf"]);
}

#[test]
fn lp_s_request_id_is_said() {
    assert_eq!(
        request("request id is Office-12 (1 file(s))\n"),
        Some("Office-12".into())
    );
    assert_eq!(request(""), None);
}

#[test]
fn the_pdf_is_handed_to_lp_and_its_request_said() {
    let dir = tempfile::tempdir().unwrap();
    let lp = fake_lp(dir.path());
    let said = print_with(&[lp.as_os_str()], b"%PDF-1.7 a", &Job::new("talk.pdf")).unwrap();
    assert_eq!(said, "sent to the printer: Office-7");
    assert_eq!(
        std::fs::read(dir.path().join("printed.pdf")).unwrap(),
        b"%PDF-1.7 a"
    );
    let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
    assert!(
        args.ends_with("-t\ntalk.pdf\n"),
        "no file named, the PDF read: {args}"
    );
}

#[test]
fn with_no_lp_the_next_is_tried_and_with_none_it_is_said() {
    let dir = tempfile::tempdir().unwrap();
    let lp = fake_lp(dir.path());
    let missing = dir.path().join("missing");
    let job = Job::new("talk.pdf");
    assert!(print_with(&[missing.as_os_str(), lp.as_os_str()], b"%PDF", &job).is_ok());
    let err = print_with(&[missing.as_os_str()], b"%PDF", &job).unwrap_err();
    assert!(format!("{err:#}").contains("CUPS"), "{err:#}");
}

#[test]
fn a_refusal_from_lp_is_said() {
    let dir = tempfile::tempdir().unwrap();
    let lp = dir.path().join("lp");
    std::fs::write(
        &lp,
        "#!/bin/sh\necho 'lp: The printer or class does not exist.' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&lp, std::fs::Permissions::from_mode(0o755)).unwrap();
    let err = print_with(&[lp.as_os_str()], b"%PDF", &Job::new("a.pdf")).unwrap_err();
    assert!(format!("{err:#}").contains("does not exist"), "{err:#}");
}

#[test]
fn paper_is_read_as_cups_names_it() {
    assert_eq!(paper("Letter"), Some((612., 792.)));
    assert_eq!(paper("A4"), Some((595., 842.)));
    assert_eq!(paper("w288h432"), Some((288., 432.)));
    assert_eq!(paper("Custom.200x300"), Some((200., 300.)));
    assert_eq!(paper("Tabloid.Fullbleed"), Some((792., 1224.)));
    assert_eq!(paper("Nonsense"), None);
}

#[test]
fn the_printer_s_paper_is_its_page_size_marked_default() {
    let options = "Duplex/2-Sided Printing: None *DuplexNoTumble DuplexTumble\n\
                   PageSize/Media Size: A4 *Letter Legal\n";
    assert_eq!(default_paper(options), Some("Letter".into()));
    assert_eq!(default_paper("Duplex: *None\n"), None);
}

#[test]
fn a_page_is_put_on_the_paper_as_the_scale_has_it() {
    let letter = (612., 792.);
    // Smaller than the paper: as it is, in the middle, unless fitted.
    let card = (306., 396.);
    let (sheet, [x, y, w, h]) = placed(card, letter, Scale::Auto);
    assert_eq!((sheet, w, h), (letter, 306., 396.));
    assert_eq!((x, y), (153., 198.));
    let (_, [_, _, w, _]) = placed(card, letter, Scale::Fit);
    assert_eq!(w, 612.);
    // Larger: shrunk, unless asked as it is.
    let tabloid = (792., 1224.);
    let (_, [_, _, w, h]) = placed(tabloid, letter, Scale::Auto);
    assert!((h - 792.).abs() < 0.01 && w < 612., "{w} {h}");
    let (_, [x, _, w, _]) = placed(tabloid, letter, Scale::Actual);
    assert_eq!((x, w), (-90., 792.));
    // A wide page is printed across the sheet.
    let slide = (453.5, 255.1);
    let (sheet, [_, _, w, _]) = placed(slide, letter, Scale::Fit);
    assert_eq!(sheet, (792., 612.));
    assert!((w - 792.).abs() < 0.01);
}

#[test]
fn the_pages_picked_are_listed_as_they_print() {
    assert_eq!(picked(None, 4), [0, 1, 2, 3]);
    // As CUPS prints them: in the document's order, each once.
    assert_eq!(picked(Some(&ranges("3-4,1,3", 4)), 4), [0, 2, 3]);
    assert_eq!(picked(Some(&ranges("2", 4)), 4), [1]);
}

#[test]
fn the_scale_is_sent_unless_left_to_the_printer() {
    let fit = Job {
        scale: Scale::Fit,
        ..Job::new("a.pdf")
    };
    assert_eq!(args(&fit, &LP), ["-o", "print-scaling=fit", "-t", "a.pdf"]);
    let actual = Job {
        scale: Scale::Actual,
        ..Job::new("a.pdf")
    };
    assert_eq!(
        args(&actual, &LP),
        ["-o", "print-scaling=none", "-t", "a.pdf"]
    );
}

#[test]
fn lpr_is_asked_by_its_own_flags_whatever_the_title() {
    let job = Job {
        printer: Some("Office".into()),
        copies: 2,
        ..Job::new("-t")
    };
    assert_eq!(args(&job, &LPR), ["-P", "Office", "-#", "2", "-T", "-t"]);
}

#[test]
fn pages_are_sent_in_order_overlaps_merged_as_cups_needs_them() {
    assert_eq!(said("9, 3-7", 0, 10), Ok(Some("3-7,9".into())));
    assert_eq!(said("3-7, 5", 0, 10), Ok(Some("3-7".into())));
    assert_eq!(said("4-6, 7, 2", 0, 10), Ok(Some("2,4-7".into())));
}

#[test]
fn wide_pages_two_sided_turn_on_the_short_edge() {
    let wide = Job {
        sides: Sides::Two,
        wide: true,
        ..Job::new("a.pdf")
    };
    let args = args(&wide, &LP);
    assert!(
        args.iter().any(|a| a == "sides=two-sided-short-edge"),
        "{args:?}"
    );
}

#[test]
fn the_pages_picked_say_whether_it_is_wide() {
    let sizes = [(800., 450.), (800., 450.), (612., 792.)];
    assert!(wide(&sizes, None));
    assert!(!wide(&sizes, Some(&ranges("3", 3))));
}
