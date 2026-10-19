use super::*;

/// The model, as `OHP_TEST_EMBED` names its directory: too large to be
/// fetched for a test, so this runs only when asked for, with `--ignored`.
#[test]
#[ignore = "needs all-MiniLM-L6-v2 downloaded, its directory named by OHP_TEST_EMBED"]
fn sentences_that_mean_the_same_are_near() {
    let dir = std::env::var_os("OHP_TEST_EMBED").expect("OHP_TEST_EMBED names a directory");
    let embedder = Embedder::load(Path::new(&dir)).unwrap();
    let sentences = [
        "Cells were sorted by the genes they express.",
        "Cells were grouped by their gene expression.",
        "The conference dinner was held by the river.",
    ];
    let mut steps = 0;
    let vectors = embedder
        .embed(&sentences, |_| {
            steps += 1;
            true
        })
        .unwrap()
        .unwrap();
    assert_eq!((vectors.len(), vectors[0].len(), steps), (3, 384, 1));
    let cosine = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
    assert!((cosine(&vectors[0], &vectors[0]) - 1.).abs() < 1e-4);
    let (near, far) = (
        cosine(&vectors[0], &vectors[1]),
        cosine(&vectors[0], &vectors[2]),
    );
    assert!(near > far + 0.3, "{near} {far}");
    assert_eq!(embedder.embed(&sentences, |_| false).unwrap(), None);
}
