//! A notes folder with many notes and real-like text, for speed tests and benches.
//!
//! The text draws its words from a fixed vocabulary of [`WORDS`] words, the most common
//! ones most often, as in real text. So many words share a prefix, and the search lists of a
//! short prefix grow long, as they do with real notes. A fixed seed gives the same folder
//! on each run.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::Path;

/// The notes in the folder.
pub const NOTES: usize = 5_000;
/// The subfolders at the root. Each note is in one of them.
pub const FOLDERS: usize = 50;
/// The tags `tag<n>` and the tags `area/tag<n>`. Each note has one of each.
pub const TAGS: usize = 100;
/// The words of the vocabulary.
pub const WORDS: usize = 40_000;

/// Common words at the top of the vocabulary. The rest are made up.
const COMMON: &[&str] = &[
    "the",
    "of",
    "and",
    "a",
    "to",
    "in",
    "is",
    "it",
    "that",
    "for",
    "on",
    "with",
    "as",
    "was",
    "be",
    "at",
    "this",
    "have",
    "from",
    "or",
    "by",
    "not",
    "but",
    "what",
    "all",
    "we",
    "when",
    "your",
    "can",
    "there",
    "use",
    "an",
    "each",
    "which",
    "do",
    "how",
    "if",
    "will",
    "up",
    "other",
    "about",
    "out",
    "many",
    "then",
    "them",
    "so",
    "some",
    "would",
    "make",
    "like",
    "time",
    "work",
    "home",
    "note",
    "plan",
    "week",
    "idea",
    "list",
    "read",
    "write",
    "meeting",
    "project",
    "people",
    "day",
    "way",
    "good",
    "new",
    "first",
    "last",
    "long",
    "little",
    "own",
    "right",
    "big",
    "high",
    "small",
    "large",
    "next",
    "early",
    "young",
    "important",
    "few",
    "public",
    "bad",
    "same",
    "able",
    "world",
    "life",
    "hand",
    "part",
    "child",
    "eye",
    "woman",
    "place",
    "case",
    "point",
    "group",
    "problem",
    "fact",
];

const ONSETS: &[&str] = &[
    "b", "c", "d", "f", "g", "h", "j", "k", "l", "m", "n", "p", "r", "s", "t", "v", "w", "z", "br",
    "ch", "cl", "dr", "fl", "gr", "pl", "pr", "sh", "sl", "st", "th", "tr", "wh", "wr", "",
];
/// The syllables of a made-up word, so most words have one or two.
const SYLLABLES: [u32; 6] = [1, 1, 2, 2, 2, 3];
const VOWELS: &[&str] = &["a", "e", "i", "o", "u", "ai", "ea", "ee", "oo", "ou", "y"];
const CODAS: &[&str] = &[
    "", "", "", "n", "r", "s", "t", "l", "m", "nd", "st", "ck", "ng",
];

/// Fills the empty folder `dir` with [`NOTES`] notes in [`FOLDERS`] folders.
///
/// The note `n` is `folder <n % FOLDERS>/Note <n>.md`, with the id `N<n>` in five digits.
/// It has two tags, note links to the next two notes, a link to a missing file, and about
/// 2 KB of text in two paragraphs.
///
/// # Errors
///
/// Any error of a folder or file write.
pub fn notes_folder(dir: &Path) -> io::Result<()> {
    for folder in 0..FOLDERS {
        fs::create_dir(dir.join(format!("folder {folder}")))?;
    }
    let mut random = Random(0x484f_4c4d);
    let words = Vocabulary::new(&mut random);
    for note in 0..NOTES {
        let text = format!(
            "---\nid: N{note:05}\ntags: [tag{}, area/tag{}]\n---\n# Note {note}\n\n{}\n\
            See [this](holm:N{:05}) and [that](holm:N{:05}), and [a file](attachments/f{note}.pdf).\n\n{}\n",
            note % TAGS,
            (note / 7) % TAGS,
            words.paragraph(&mut random, 1_000),
            (note + 1) % NOTES,
            (note + 2) % NOTES,
            words.paragraph(&mut random, 1_000),
        );
        let path = dir
            .join(format!("folder {}", note % FOLDERS))
            .join(format!("Note {note}.md"));
        fs::write(path, text)?;
    }
    Ok(())
}

/// The words and the share of text each one gets. The word at rank `r` comes
/// `1 / (r + 1)` as often as the first one.
struct Vocabulary {
    words: Vec<String>,
    /// The sum of the shares up to and with each word.
    sums: Vec<f64>,
}

impl Vocabulary {
    fn new(random: &mut Random) -> Self {
        let mut words: Vec<String> = COMMON.iter().map(|&word| word.to_owned()).collect();
        let mut known: HashSet<String> = words.iter().cloned().collect();
        while words.len() < WORDS {
            let mut word = String::new();
            for _ in 0..SYLLABLES[random.below(6) as usize] {
                word.push_str(random.pick(ONSETS));
                word.push_str(random.pick(VOWELS));
                word.push_str(random.pick(CODAS));
            }
            if word.len() > 2 && known.insert(word.clone()) {
                words.push(word);
            }
        }
        let mut sum = 0.;
        let sums = (1..=u32::try_from(WORDS).unwrap_or(u32::MAX))
            .map(|rank| {
                sum += 1. / f64::from(rank);
                sum
            })
            .collect();
        Self { words, sums }
    }

    fn word(&self, random: &mut Random) -> &str {
        let at = random.fraction() * self.sums.last().copied().unwrap_or_default();
        let rank = self.sums.partition_point(|&sum| sum < at);
        &self.words[rank.min(self.words.len() - 1)]
    }

    /// Sentences of 6 to 17 words, until the text has at least `bytes` bytes.
    fn paragraph(&self, random: &mut Random, bytes: usize) -> String {
        let mut text = String::new();
        while text.len() < bytes {
            if !text.is_empty() {
                text.push(' ');
            }
            let count = 6 + random.below(12);
            for at in 0..count {
                let word = self.word(random);
                if at == 0 {
                    let mut chars = word.chars();
                    text.extend(chars.next().map(|first| first.to_ascii_uppercase()));
                    text.push_str(chars.as_str());
                } else {
                    let _ = write!(text, " {word}");
                }
            }
            text.push('.');
        }
        text
    }
}

/// `SplitMix64`: a small, fast generator. It is not for secrets.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number from 0 up to `n`, without `n`.
    fn below(&mut self, n: u32) -> u32 {
        u32::try_from(self.next() % u64::from(n)).unwrap_or(0)
    }

    /// A number from 0 up to 1, without 1.
    fn fraction(&mut self) -> f64 {
        f64::from(u32::try_from(self.next() >> 32).unwrap_or(0)) / f64::from(u32::MAX)
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        let at = usize::try_from(self.below(u32::try_from(items.len()).unwrap_or(u32::MAX)))
            .unwrap_or(0);
        items[at]
    }
}
