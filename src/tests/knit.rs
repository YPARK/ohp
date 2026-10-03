use super::*;
use crate::fixture::{gone, marked_sleep, r_here, write_text};
use std::sync::Arc;
use std::sync::atomic::Ordering;

#[test]
fn chunks_of_any_engine_are_run_and_plain_blocks_are_not() {
    assert!(has_chunks("```{python}\nprint(1)\n```\n", false));
    assert!(has_chunks("``` {bash, echo=FALSE}\nls\n```\n", false));
    assert!(has_chunks("```{r}\n1\n```\n", true));
    assert!(!has_chunks("```python\nprint(1)\n```\n", false));
    assert!(!has_chunks("```{.python}\nprint(1)\n```\n", false));
    assert!(!has_chunks("Inline `r 1 + 1` in markdown.\n", false));
    assert!(has_chunks("Inline `r 1 + 1` in R Markdown.\n", true));
}

/// `path` knitted, or the test fails saying why not.
fn knitted(path: &Path) -> (TempDir, String) {
    match knit(path, &AtomicBool::new(false)) {
        Knitted::Done(dir, md) => (dir, md),
        Knitted::Failed(why) => panic!("{why}"),
        Knitted::Stopped => panic!("stopped"),
    }
}

#[test]
fn knitr_runs_chunks_beside_the_file_and_keeps_plots_apart() {
    if !r_here() {
        eprintln!("skipped: no R with knitr");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("data.csv"), "x\n1\n2\n").unwrap();
    let path = write_text(
        dir.path(),
        "a.Rmd",
        "```{r}\nsum(read.csv('data.csv')$x) * 7\n```\n\n```{r p}\nplot(1:3)\n```\n",
    );
    let (out, md) = knitted(&path);
    assert!(md.contains("## [1] 21"), "{md}");
    let figures = out.path().join("figure");
    assert!(std::fs::read_dir(&figures).unwrap().count() > 0);
    assert!(md.contains(&*figures.to_string_lossy()), "{md}");
    assert!(!dir.path().join("figure").exists());
}

#[test]
fn knitr_runs_bash_chunks_in_plain_markdown() {
    if !r_here() {
        eprintln!("skipped: no R with knitr");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("here.txt"), "").unwrap();
    let path = write_text(dir.path(), "a.md", "```{bash}\nls *.txt\n```\n");
    let (_, md) = knitted(&path);
    assert!(md.contains("## here.txt"), "{md}");
}

#[test]
fn a_stopped_knit_ends_r_and_what_its_chunks_started() {
    if !r_here() {
        eprintln!("skipped: no R with knitr");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let sleep = marked_sleep(61);
    let path = write_text(
        dir.path(),
        "slow.Rmd",
        &format!("```{{r}}\nsystem(\"{sleep}\")\n```\n"),
    );
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        stopping.store(true, Ordering::Relaxed);
    });
    let start = std::time::Instant::now();
    assert!(matches!(knit(&path, &stop), Knitted::Stopped));
    assert!(start.elapsed().as_secs() < 10);
    assert!(gone(&sleep), "what the chunk started outlived it");
}
