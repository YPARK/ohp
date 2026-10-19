use super::*;

#[test]
fn a_repo_is_kept_where_hugging_face_keeps_it() {
    let cache = Path::new("/cache");
    assert_eq!(
        dir(cache, "Qwen/Qwen2.5-3B-Instruct-GGUF"),
        Path::new("/cache/models--Qwen--Qwen2.5-3B-Instruct-GGUF")
    );
    let remote = Remote {
        path: "m.gguf".into(),
        size: 3,
        blob: "abc".into(),
    };
    let blob = blob(cache, "a/b", &remote);
    assert_eq!(blob, Path::new("/cache/models--a--b/blobs/abc"));
    assert_eq!(
        incomplete(&blob),
        Path::new("/cache/models--a--b/blobs/abc.incomplete")
    );
}

#[test]
fn the_hubs_listing_is_read() {
    assert_eq!(
        commit(r#"{"id":"a/b","sha":"c0ffee","siblings":[]}"#),
        Ok("c0ffee".into())
    );
    assert!(commit(r#"{"error":"Repository not found"}"#).is_err());
    let tree = r#"[
        {"type":"directory","oid":"d1","size":0,"path":"sub"},
        {"type":"file","oid":"g1","size":612,"path":"config.json"},
        {"type":"file","oid":"g2","size":135,"path":"m.gguf",
         "lfs":{"oid":"sha256hash","size":2104932768,"pointerSize":135}}
    ]"#;
    assert_eq!(
        files(tree),
        Ok(vec![
            Remote {
                path: "config.json".into(),
                size: 612,
                blob: "g1".into(),
            },
            Remote {
                path: "m.gguf".into(),
                size: 2_104_932_768,
                blob: "sha256hash".into(),
            },
        ])
    );
}

#[test]
fn why_a_repo_cannot_be_had_is_said_plainly() {
    let why = |curl: &str| trouble("meta/llama", curl);
    assert!(why("The requested URL returned error: 401").contains("HF_TOKEN"));
    assert!(why("The requested URL returned error: 404").starts_with("no meta/llama"));
    assert!(why("Could not resolve host").contains("cannot reach"));
}

/// A cache with `repo`'s `files` at `commit`, each a blob its snapshot
/// links to, as Hugging Face's libraries keep them.
#[cfg(unix)]
fn keep(cache: &Path, repo: &str, commit: &str, files: &[&str]) {
    for (i, file) in files.iter().enumerate() {
        let remote = Remote {
            path: file.to_string(),
            size: 1,
            blob: format!("blob{i}"),
        };
        let blob = blob(cache, repo, &remote);
        std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
        std::fs::write(&blob, b"x").unwrap();
        place(
            &dir(cache, repo).join("snapshots").join(commit).join(file),
            &blob,
        )
        .unwrap();
    }
}

#[cfg(unix)]
#[test]
fn files_kept_are_found_through_their_snapshots_links() {
    let cache = tempfile::tempdir().unwrap();
    let cache = cache.path();
    keep(cache, "a/b", "old", &["m.gguf"]);
    keep(cache, "a/b", "new", &["m.gguf", "sub/n.gguf"]);
    std::fs::create_dir_all(dir(cache, "a/b").join("refs")).unwrap();
    std::fs::write(dir(cache, "a/b").join("refs/main"), "new\n").unwrap();
    let found = cached(cache, "a/b", "m.gguf").unwrap();
    assert!(found.ends_with("snapshots/new/m.gguf"));
    assert_eq!(std::fs::read(&found).unwrap(), b"x");
    let nested = cached(cache, "a/b", "sub/n.gguf").unwrap();
    assert_eq!(
        std::fs::read(nested).unwrap(),
        b"x",
        "linked from a directory down"
    );
    assert_eq!(cached(cache, "a/b", "other.gguf"), None);
    assert_eq!(cached(cache, "c/d", "m.gguf"), None);
    assert_eq!(
        kept(cache),
        vec![
            ("a/b".to_string(), "m.gguf".to_string()),
            ("a/b".to_string(), "sub/n.gguf".to_string()),
        ]
    );
}

#[cfg(unix)]
#[test]
fn a_link_to_a_blob_gone_is_not_kept() {
    let cache = tempfile::tempdir().unwrap();
    let cache = cache.path();
    keep(cache, "a/b", "c", &["m.gguf"]);
    std::fs::remove_file(dir(cache, "a/b").join("blobs/blob0")).unwrap();
    assert_eq!(cached(cache, "a/b", "m.gguf"), None);
    assert!(kept(cache).is_empty());
}

#[test]
#[ignore = "asks Hugging Face's Hub"]
fn a_repo_is_listed_by_the_hub() {
    let listing = list(crate::embed::REPO, &AtomicBool::new(false)).unwrap();
    assert_eq!(listing.commit.len(), 40);
    let weights = (listing.files.iter())
        .find(|f| f.path == "model.safetensors")
        .unwrap();
    assert_eq!(weights.blob.len(), 64, "its LFS hash");
    assert!(weights.size > 80_000_000);
    let missing = list("ohp-no-such/repo-here", &AtomicBool::new(false));
    assert!(missing.unwrap_err().contains("ohp-no-such/repo-here"));
}

#[test]
#[ignore = "downloads from Hugging Face's Hub"]
fn a_file_is_downloaded_into_the_cache_and_found_there() {
    let cache = tempfile::tempdir().unwrap();
    let cache = cache.path();
    let repo = crate::embed::REPO;
    let stop = AtomicBool::new(false);
    let listing = list(repo, &stop).unwrap();
    let config = (listing.files.iter())
        .find(|f| f.path == "config.json")
        .unwrap();
    let path = download(cache, repo, &listing.commit, config, &stop).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().len(), config.size);
    assert_eq!(cached(cache, repo, "config.json"), Some(path));
    assert!(!incomplete(&blob(cache, repo, config)).exists());
    let main = std::fs::read_to_string(dir(cache, repo).join("refs/main")).unwrap();
    assert_eq!(main, listing.commit);
}
