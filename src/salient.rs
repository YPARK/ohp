//! The sentences of a document most central to it, the ones to sum it up
//! from: each is scored by how like the whole document it is, the cosine
//! between its vector and the sum of all of theirs. The vectors are what
//! the sentences mean, as an embedding model has them, or else their
//! words, as TF-IDF weighs them. A sentence is not picked if it says nearly
//! what one picked already does, as an abstract's sentences said again in
//! the introduction.
//!
//! Central is not all a takeaway is: a method's details are as central.
//! As extractive summaries long have, a sentence scores more on the first
//! page, where an abstract or a title slide sums the rest up, and where it
//! says what was found, as `we show` or `our results`; and less where it
//! points to a figure or a table, as a result's details do.
//!
//! Headings and labels are too short to be picked, and garbled text too
//! long; nor is a reference, known by its years, or its DOI or link, nor a
//! piece of a sentence, as a caption broken across lines, begun in lower
//! case, nor a running head or foot, said on page after page.

use std::collections::{HashMap, HashSet};

/// Words too common to tell sentences apart.
const STOP: &[&str] = &[
    "about", "above", "after", "again", "against", "all", "also", "among", "and", "any", "are",
    "because", "been", "before", "being", "below", "between", "both", "but", "can", "could", "did",
    "does", "doing", "down", "during", "each", "few", "for", "from", "further", "had", "has",
    "have", "having", "here", "how", "into", "its", "itself", "just", "may", "more", "most",
    "much", "must", "not", "now", "off", "once", "only", "other", "our", "ours", "out", "over",
    "own", "same", "should", "show", "shown", "shows", "some", "such", "than", "that", "the",
    "their", "them", "then", "there", "these", "they", "this", "those", "through", "thus", "too",
    "under", "until", "upon", "use", "used", "using", "very", "was", "were", "what", "when",
    "where", "which", "while", "who", "whom", "why", "will", "with", "within", "without", "would",
    "you", "your",
];
/// Fewest and most words a sentence picked has.
const FEWEST_WORDS: usize = 6;
const MOST_WORDS: usize = 90;
/// Fewest pages a running head or foot is on.
const RUNNING: usize = 3;
/// Least cosine between two sentences that say nearly the same, by their
/// words and by what they mean.
const SAME_WORDS: f32 = 0.7;
const SAME_MEANING: f32 = 0.9;
/// Bytes a sentence is given to the model in, past its own: its number and
/// its page.
const NUMBERING: usize = 16;
/// Phrases that say what was found, in lower case.
const CUES: &[&str] = &[
    "here we",
    "we show",
    "we find",
    "we found",
    "we demonstrate",
    "we propose",
    "we present",
    "we report",
    "we identify",
    "we conclude",
    "our results",
    "our findings",
    "results show",
    "results suggest",
    "in conclusion",
    "in summary",
    "taken together",
    "this suggests",
    "these results",
    "these findings",
    "we reveal",
];
/// What a sentence's score is multiplied by on the first page, where it
/// says what was found, and where it points to a figure or a table.
const LEAD: f32 = 1.3;
const CUED: f32 = 1.5;
const POINTING: f32 = 0.7;

/// A sentence as its words, and how much each weighs.
type Sparse = HashMap<String, f32>;

/// Sentences as vectors, each of length one: by their words, as TF-IDF
/// weighs them, or by what they mean, as an embedding model has them.
pub enum Vectors {
    Words(Vec<Sparse>),
    Meanings(Vec<Vec<f32>>),
}

impl Vectors {
    /// The cosine between sentences `i` and `j`.
    fn cosine(&self, i: usize, j: usize) -> f32 {
        match self {
            Vectors::Words(v) => dot(&v[i], &v[j]),
            Vectors::Meanings(v) => dense_dot(&v[i], &v[j]),
        }
    }

    /// The cosine between each sentence and their sum.
    fn centrality(&self) -> Vec<f32> {
        match self {
            Vectors::Words(vectors) => {
                let mut centre = Sparse::new();
                for v in vectors {
                    for (word, w) in v {
                        *centre.entry(word.clone()).or_default() += w;
                    }
                }
                let centre = normalised(centre);
                vectors.iter().map(|v| dot(v, &centre)).collect()
            }
            Vectors::Meanings(vectors) => {
                let len = vectors.first().map_or(0, Vec::len);
                let mut centre = vec![0f32; len];
                for v in vectors {
                    centre.iter_mut().zip(v).for_each(|(c, x)| *c += x);
                }
                let norm = centre
                    .iter()
                    .map(|c| c * c)
                    .sum::<f32>()
                    .sqrt()
                    .max(f32::EPSILON);
                vectors
                    .iter()
                    .map(|v| dense_dot(v, &centre) / norm)
                    .collect()
            }
        }
    }

    /// The least cosine of two sentences that say nearly the same: words
    /// shared count for less than meaning shared.
    fn same(&self) -> f32 {
        match self {
            Vectors::Words(_) => SAME_WORDS,
            Vectors::Meanings(_) => SAME_MEANING,
        }
    }
}

/// The sentences of `pages` that may be picked, each its page and its
/// sentence there, in order.
pub fn candidates(pages: &[Vec<String>]) -> Vec<(usize, usize)> {
    let running = running(pages);
    let mut candidates = Vec::new();
    for (p, said) in pages.iter().enumerate() {
        for (k, sentence) in said.iter().enumerate() {
            if (FEWEST_WORDS..=MOST_WORDS).contains(&sentence.split_whitespace().count())
                && !reference(sentence)
                && !sentence.starts_with(char::is_lowercase)
                && !running.contains(&undigited(sentence))
                && !words(sentence).is_empty()
            {
                candidates.push((p, k));
            }
        }
    }
    candidates
}

/// `sentences` as TF-IDF weighs their words.
pub fn tfidf(sentences: &[&str]) -> Vectors {
    let words: Vec<Vec<String>> = sentences.iter().map(|s| words(s)).collect();
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for words in &words {
        for word in words.iter().collect::<HashSet<_>>() {
            *seen.entry(word).or_default() += 1;
        }
    }
    let n = sentences.len() as f32;
    let idf = |word: &str| ((n + 1.) / (seen[word] as f32 + 1.)).ln() + 1.;
    let vectors = (words.iter())
        .map(|words| {
            let mut v = Sparse::new();
            for word in words {
                *v.entry(word.clone()).or_default() += idf(word);
            }
            normalised(v)
        })
        .collect();
    Vectors::Words(vectors)
}

/// The sentences of `pages` most central to them, as TF-IDF weighs their
/// words, as `pick_from` picks them.
#[cfg(test)]
pub fn pick(pages: &[Vec<String>], budget: usize) -> Vec<(usize, usize)> {
    let candidates = candidates(pages);
    let said: Vec<&str> = candidates
        .iter()
        .map(|&(p, k)| pages[p][k].as_str())
        .collect();
    pick_from(pages, &candidates, &tfidf(&said), budget)
}

/// Of `candidates`, sentences of `pages` as `candidates` gives them, and
/// `vectors`, theirs, the most central, each its page and its sentence
/// there, as many as `budget` bytes of them hold, in the order they come.
pub fn pick_from(
    pages: &[Vec<String>],
    candidates: &[(usize, usize)],
    vectors: &Vectors,
    budget: usize,
) -> Vec<(usize, usize)> {
    let said = |i: usize| pages[candidates[i].0][candidates[i].1].as_str();
    let scores: Vec<f32> = (vectors.centrality().into_iter().enumerate())
        .map(|(i, score)| score * weight(said(i), candidates[i].0 == 0))
        .collect();
    let mut ranked: Vec<usize> = (0..scores.len()).collect();
    ranked.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
    let (mut picked, mut used) = (Vec::new(), 0);
    for i in ranked {
        let len = said(i).len() + NUMBERING;
        let again = (picked.iter()).any(|&j| vectors.cosine(i, j) >= vectors.same());
        if used + len <= budget && !again {
            used += len;
            picked.push(i);
        }
    }
    let mut picked: Vec<(usize, usize)> = picked.into_iter().map(|i| candidates[i]).collect();
    picked.sort_unstable();
    picked
}

/// Sentences on `RUNNING` pages or more, but for their digits, as a
/// running head or foot is, with its page number.
fn running(pages: &[Vec<String>]) -> HashSet<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    for said in pages {
        for sentence in said.iter().map(|s| undigited(s)).collect::<HashSet<_>>() {
            *seen.entry(sentence).or_default() += 1;
        }
    }
    (seen.into_iter())
        .filter(|&(_, n)| n >= RUNNING)
        .map(|(s, _)| s)
        .collect()
}

fn undigited(sentence: &str) -> String {
    sentence.chars().filter(|c| !c.is_ascii_digit()).collect()
}

/// What the score of `sentence` is multiplied by, as it is on the first
/// page, says what was found, or points to a figure or a table.
fn weight(sentence: &str, first: bool) -> f32 {
    let lower = sentence.to_lowercase();
    let mut weight = 1.;
    if first {
        weight *= LEAD;
    }
    if CUES.iter().any(|cue| lower.contains(cue)) {
        weight *= CUED;
    }
    let pointing = [
        "figure",
        "figures",
        "fig",
        "figs",
        "table",
        "tables",
        "supplement",
        "supplementary",
    ];
    if said(&lower).any(|w| pointing.contains(&w)) {
        weight *= POINTING;
    }
    weight
}

/// The words of `lower`, a sentence in lower case, as they are written.
fn said(lower: &str) -> impl Iterator<Item = &str> {
    lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
}

/// The words of `sentence` that tell it apart: of three letters or more,
/// in lower case, but for the common ones.
fn words(sentence: &str) -> Vec<String> {
    (sentence.split(|c: char| !c.is_alphanumeric()))
        .filter(|w| w.chars().count() >= 3 && w.chars().any(char::is_alphabetic))
        .map(str::to_lowercase)
        .filter(|w| !STOP.contains(&w.as_str()))
        .collect()
}

/// Whether `sentence` is a reference's: it gives two years or more, as
/// `2019`, or a DOI or a link. A sentence citing one work is not.
fn reference(sentence: &str) -> bool {
    let years = (sentence.split(|c: char| !c.is_ascii_digit()))
        .filter(|n| n.len() == 4 && (n.starts_with("19") || n.starts_with("20")))
        .count();
    let lower = sentence.to_lowercase();
    years >= 2 || said(&lower).any(|w| ["doi", "http", "https"].contains(&w))
}

fn normalised(mut v: Sparse) -> Sparse {
    let norm = v.values().map(|w| w * w).sum::<f32>().sqrt();
    if norm > 0. {
        v.values_mut().for_each(|w| *w /= norm);
    }
    v
}

fn dot(a: &Sparse, b: &Sparse) -> f32 {
    let (a, b) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    a.iter()
        .filter_map(|(word, w)| Some(w * b.get(word)?))
        .sum()
}

fn dense_dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

#[cfg(test)]
#[path = "tests/salient.rs"]
mod tests;
