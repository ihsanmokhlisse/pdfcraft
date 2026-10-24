//! Romanian diacritics after recognition (`ro`).
//!
//! Neither recogniser writes every Romanian letter: the default model has no `ă â î ș ț` at all
//! (it reads `ă` and `â` as `?`, and `ș ț î` as `s t i`), and the Latin model has `â î ş` but no
//! `ă`, `ț` or the comma-below `ș`. So a Romanian page comes out as `P?n? m?ine, la scoala`.
//!
//! [`Lexicon::restore_text`] puts the letters back word by word from a frequency list of Romanian
//! word forms (`ro-words.txt`, fetched with the models): each recognised word is matched against
//! the forms that differ from it only in those letters, and a letter is changed only when the
//! likely forms agree on it. `scoala` has `școală` and `școala` (and the verb `scoală`) to choose
//! from, so it becomes `școala`: the `ș` is certain, the last letter isn't, and the reading stays.
//! A `?` inside a word is a letter the model could not write, so it always becomes one. The
//! cedilla letters some models and old documents use (`ş ţ`) become the comma-below ones.

use std::collections::HashMap;

/// The list file, as named in ATTRIBUTION.toml (beside the models).
pub const WORDS: &str = "ro-words.txt";

/// Most forms read from the list, and its largest accepted size: the file is input like any
/// other, and the shipped one has 50,000 lines (about 650 KB).
const MAX_FORMS: usize = 400_000;
const MAX_BYTES: usize = 16 * 1024 * 1024;
/// Longest word looked up (and longest form kept): longer runs are not Romanian words.
const MAX_WORD: usize = 40;
/// At most this many `?` are tried in one word (each has two readings, `a` or `i`).
const MAX_UNKNOWN: usize = 4;
/// A form is a likely reading when its count is at least the best one's divided by this.
const LIKELY: u64 = 4;
/// The list comes from subtitles, which are often typed without diacritics: this share of the
/// forms with diacritics is taken off the plain form's count before comparing them.
const UNMARKED_PER_MILLE: u64 = 150;

/// Romanian word forms with diacritics, by their plain spelling, with how often each was seen.
#[derive(Debug, Default)]
pub struct Lexicon {
    forms: HashMap<String, Vec<(String, u64)>>,
}

/// The letter a Romanian letter is written as without its diacritic.
fn plain(c: char) -> char {
    match c {
        'ă' | 'â' => 'a',
        'î' => 'i',
        'ș' => 's',
        'ț' => 't',
        _ => c,
    }
}

fn is_marked(c: char) -> bool {
    matches!(c, 'ă' | 'â' | 'î' | 'ș' | 'ț')
}

/// A recognised character, lower case, as Romanian: the cedilla letters are the comma-below
/// ones, and a Latin letter with another accent (`ã`, `á`, `š`) is that base letter, unsure.
/// Returns the letter and whether it is unsure.
fn reading(c: char) -> (char, bool) {
    match c {
        'ş' => ('ș', false),
        'ţ' => ('ț', false),
        'ã' | 'á' | 'à' | 'ä' | 'å' | 'ā' => ('a', true),
        'í' | 'ì' | 'ï' => ('i', true),
        'š' | 'ś' => ('s', true),
        'ť' => ('t', true),
        _ => (c, false),
    }
}

fn is_romanian_letter(c: char) -> bool {
    c.is_ascii_lowercase() || is_marked(c)
}

impl Lexicon {
    /// Parse a frequency list: one `form count` per line (the count is optional). Lines that
    /// aren't a lower-case Romanian word are skipped; the cedilla spellings count as the
    /// comma-below ones.
    pub fn parse(text: &str) -> Lexicon {
        let mut counts: HashMap<String, u64> = HashMap::new();
        let text = text.get(..text.len().min(MAX_BYTES)).unwrap_or_default();
        for line in text.lines().take(MAX_FORMS) {
            let mut fields = line.split_whitespace();
            let Some(word) = fields.next() else { continue };
            let count = fields.next().and_then(|n| n.parse::<u64>().ok()).unwrap_or(1);
            let form: String = word.chars().flat_map(char::to_lowercase).map(|c| reading(c).0).collect();
            if form.chars().count() > MAX_WORD || !form.chars().all(|c| is_romanian_letter(c) || c == '-') || !form.chars().any(is_romanian_letter) {
                continue;
            }
            let slot = counts.entry(form).or_default();
            *slot = slot.saturating_add(count);
        }
        let mut forms: HashMap<String, Vec<(String, u64)>> = HashMap::new();
        for (form, count) in counts {
            forms.entry(form.chars().map(plain).collect()).or_default().push((form, count));
        }
        // Only spellings that have a form with diacritics can change anything.
        forms.retain(|_, v| v.iter().any(|(f, _)| f.chars().any(is_marked)));
        for v in forms.values_mut() {
            v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        }
        Lexicon { forms }
    }

    /// Read [`WORDS`] from `dir`, if it is there.
    pub fn load(dir: &std::path::Path) -> Option<Lexicon> {
        let path = dir.join(WORDS);
        let size = std::fs::metadata(&path).ok()?.len();
        if size > MAX_BYTES as u64 {
            return None;
        }
        let bytes = std::fs::read(&path).ok()?;
        Some(Lexicon::parse(&String::from_utf8_lossy(&bytes)))
    }

    pub fn is_empty(&self) -> bool {
        self.forms.is_empty()
    }

    /// `text` (a recognised word or line) with Romanian diacritics restored in each word.
    pub fn restore_text(&self, text: &str) -> String {
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::with_capacity(text.len());
        let mut i = 0;
        while let Some(&c) = chars.get(i) {
            if !is_word_char(c) {
                out.push(c);
                i += 1;
                continue;
            }
            let start = i;
            while chars.get(i).is_some_and(|&c| is_word_char(c) || (c == '-' && chars.get(i + 1).copied().is_some_and(is_word_char))) {
                i += 1;
            }
            let token: Vec<char> = chars.get(start..i).unwrap_or_default().to_vec();
            out.push_str(&self.restore_token(&token));
        }
        out
    }

    /// One run of letters, `?` and inner hyphens.
    fn restore_token(&self, token: &[char]) -> String {
        // A `?` at the end is a question mark unless a word ends there.
        if let Some((&'?', head)) = token.split_last()
            && head.iter().any(|c| c.is_alphabetic())
            && !head.contains(&'?')
        {
            if let Some(word) = self.lookup(token) {
                return word;
            }
            let mut s = self.restore_word(head);
            s.push('?');
            return s;
        }
        if token.iter().all(|&c| c == '?') {
            return token.iter().collect();
        }
        self.restore_word(token)
    }

    /// A word, whole or (when the list doesn't have it whole) by its hyphenated parts.
    fn restore_word(&self, word: &[char]) -> String {
        if let Some(w) = self.lookup(word) {
            return w;
        }
        if word.contains(&'-') {
            return word.split(|&c| c == '-').map(|p| self.restore_word(p)).collect::<Vec<_>>().join("-");
        }
        guess(word)
    }

    /// The word with its letters restored from the list, or `None` when the list has no form of it.
    fn lookup(&self, word: &[char]) -> Option<String> {
        if word.is_empty() || word.len() > MAX_WORD {
            return None;
        }
        let lower: Vec<(char, bool)> = word.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).map(reading).collect();
        let unknown: Vec<usize> = lower.iter().enumerate().filter(|(_, (c, _))| *c == '?').map(|(i, _)| i).collect();
        if unknown.len() > MAX_UNKNOWN {
            return None;
        }
        // Every plain spelling the word could have: each `?` is an `a` (ă, â) or an `i` (î).
        let mut keys: Vec<Vec<char>> = vec![lower.iter().map(|(c, _)| plain(*c)).collect()];
        for &u in &unknown {
            keys = keys
                .into_iter()
                .flat_map(|k| {
                    ['a', 'i'].into_iter().map(move |v| {
                        let mut k = k.clone();
                        if let Some(slot) = k.get_mut(u) {
                            *slot = v;
                        }
                        k
                    })
                })
                .collect();
        }
        let mut found: Vec<(&str, u64)> = Vec::new();
        for key in keys {
            let key: String = key.into_iter().collect();
            for (form, count) in self.forms.get(&key).map(Vec::as_slice).unwrap_or_default() {
                if fits(form, &lower) {
                    found.push((form, *count));
                }
            }
        }
        if found.is_empty() {
            return None;
        }
        // The plain form's count also holds the times the others were typed without diacritics.
        let marked: u64 = found.iter().filter(|(f, _)| f.chars().any(is_marked)).map(|(_, n)| *n).fold(0, u64::saturating_add);
        let discount = marked.saturating_mul(UNMARKED_PER_MILLE) / 1000;
        let mut scored: Vec<(&str, u64)> =
            found.into_iter().map(|(f, n)| (f, if f.chars().any(is_marked) { n } else { n.saturating_sub(discount) })).collect();
        scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        let best = scored.first().map_or(0, |s| s.1);
        let likely: Vec<Vec<char>> = scored.iter().filter(|s| s.1.saturating_mul(LIKELY) >= best).map(|s| s.0.chars().collect()).collect();
        let first = likely.first()?;
        let restored: String = lower
            .iter()
            .enumerate()
            .map(|(i, (read, unsure))| {
                let agreed = likely.iter().all(|f| f.get(i) == first.get(i));
                match first.get(i) {
                    // A letter the likely forms agree on, or one the model could not write at all.
                    Some(&c) if agreed || *read == '?' || *unsure => c,
                    _ => *read,
                }
            })
            .collect();
        Some(in_case_of(word, &restored))
    }
}

/// Can `form` be what was read? A letter read with its diacritic must have it; a `?` is a
/// letter with one; an unsure accent is any form of its base letter.
fn fits(form: &str, read: &[(char, bool)]) -> bool {
    form.chars().count() == read.len()
        && form.chars().zip(read).all(|(f, &(r, unsure))| match r {
            '?' => is_marked(f) && plain(f) != 's' && plain(f) != 't',
            _ if unsure => plain(f) == r,
            _ if is_marked(r) => f == r,
            _ => plain(f) == r,
        })
}

fn is_word_char(c: char) -> bool {
    c.is_alphabetic() || c == '?'
}

/// A word the list doesn't have: only the letters the model could not write are filled in, with
/// the usual one (`î` starts a word, `â` never ends one, `ă` is the commonest), and the cedilla
/// letters become comma-below ones.
fn guess(word: &[char]) -> String {
    let last = word.len().saturating_sub(1);
    let letters: String = word
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let lower = c.to_lowercase().next().unwrap_or(c);
            match reading(lower).0 {
                '?' if i == 0 && last > 0 => 'î',
                '?' => 'ă',
                r if is_marked(r) => r,
                _ => lower,
            }
        })
        .collect();
    in_case_of(word, &letters)
}

/// `restored` (lower case) in the letter case of `word`: all capitals, a capital first, or as
/// written letter by letter otherwise.
fn in_case_of(word: &[char], restored: &str) -> String {
    let letters: Vec<char> = word.iter().copied().filter(|c| c.is_alphabetic()).collect();
    let upper = letters.len() > 1 && letters.iter().all(|c| !c.is_lowercase());
    if upper || (letters.len() == 1 && letters.iter().all(|c| c.is_uppercase()) && word.len() > 1) {
        return restored.to_uppercase();
    }
    restored
        .chars()
        .zip(word.iter().copied().chain(std::iter::repeat(' ')))
        .map(|(r, w)| if w.is_uppercase() { r.to_uppercase().collect::<String>() } else { r.to_string() })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-written counts, shaped like the real list (subtitles often lack diacritics).
    const LIST: &str = "să 7000\nsa 1300\nși 4600\nsi 1000\nşi 300\nîn 4500\nin 500\npână 300\npana 30\nmâine 90\nmaine 10\n\
        școală 45\nșcoala 17\nscoala 13\nscoală 4\nșcolii 20\nscolii 3\nfață 86\nfața 83\nfata 119\nfată 75\nțară 22\nțara 20\ntara 14\n\
        bibliotecă 9\nbiblioteca 8\ngării 5\ngarii 1\nrom 2\nromânia 9\nromania 4\nștiința 6\nstiinta 1\nînștiințare 2\nședința 8\nsedinta 1\n\
        într-o 50\nintr-o 9\ns-a 400\nfăcut 160\nfacut 22\npași 8\npasi 2\nmândri 3\nmandri 1\nca 1900\ncă 4300\nmă 1500\nma 250\n\
        va 1100\nvă 640\nstrada 19\nstradă 19\nînvață 30\ninvata 5\nînvăța 10\nînvăță 2\nnoua 5\nnouă 9\nx1y 3\nĂȘTIA 2\n";

    fn lex() -> Lexicon {
        Lexicon::parse(LIST)
    }

    #[test]
    fn question_marks_and_plain_letters_get_their_diacritics() {
        let l = lex();
        for (read, want) in [
            ("P?n? m?ine", "Până mâine"),
            ("bibliotec?.", "bibliotecă."),
            ("G?rii", "Gării"),
            ("Stiinta si tehnica", "Știința și tehnica"),
            ("f?cut pasi", "făcut pași"),
            ("in Rom?nia", "în România"),
            ("INSTIINTARE: sedinta", "ÎNȘTIINȚARE: ședința"),
            ("intr-o zi", "într-o zi"),
            ("m?ndri", "mândri"),
            ("?n", "în"),
        ] {
            assert_eq!(l.restore_text(read), want, "{read}");
        }
    }

    #[test]
    fn a_letter_changes_only_when_the_likely_forms_agree() {
        let l = lex();
        // școală / școala: the ș is certain, the last letter isn't.
        assert_eq!(l.restore_text("scoala"), "școala");
        // fata, față, fața, fată: nothing agrees, the reading stays.
        assert_eq!(l.restore_text("fata"), "fata");
        // țară, țara, tara: the ț isn't certain either (the plain form is common enough).
        assert_eq!(l.restore_text("tara"), "tara");
        // ca / că and va / vă are both common: unchanged. strada / stradă: unchanged.
        for w in ["ca", "va", "strada", "Strada"] {
            assert_eq!(l.restore_text(w), w);
        }
        // A plain spelling written mostly with diacritics is restored: să, și, mă.
        assert_eq!(l.restore_text("sa si ma"), "să și mă");
        // A `?` always becomes a letter, even where the forms disagree: învață or învăța keep
        // the î, ă and ț they share, and the reading's last letter.
        assert_eq!(l.restore_text("inv?ta"), "învăța");
    }

    #[test]
    fn cedillas_become_comma_below_and_question_marks_stay_questions() {
        let l = lex();
        assert_eq!(l.restore_text("Şcoala şi ţara"), "Școala și țara");
        assert_eq!(l.restore_text("Vii? Unde?"), "Vii? Unde?");
        assert_eq!(l.restore_text("??? ?"), "??? ?");
        // A word the list doesn't have: only the unwritable letters are filled in.
        assert_eq!(l.restore_text("Br?ila"), "Brăila");
        assert_eq!(l.restore_text("?nchis"), "închis");
        assert_eq!(l.restore_text("ţesut"), "țesut");
        assert_eq!(l.restore_text("hello world"), "hello world");
        // Another accent the Latin model has (ã) is an unsure a.
        assert_eq!(l.restore_text("pãnã"), "până");
    }

    #[test]
    fn the_list_is_read_leniently() {
        let l = lex();
        assert!(!l.is_empty());
        // Not a Romanian word, or only plain spellings: nothing to restore.
        assert!(!l.forms.contains_key("x1y"));
        assert!(!l.forms.contains_key("rom"));
        // Upper case and cedilla spellings count as the lower-case comma-below ones.
        assert!(l.forms.get("si").is_some_and(|v| v.iter().any(|(f, n)| f == "și" && *n == 4900)));
        assert!(l.forms.contains_key("astia"));
        assert!(Lexicon::parse("").is_empty());
        assert!(Lexicon::parse("\n\n  \nfoo bar baz\n").is_empty());
        assert!(Lexicon::load(std::path::Path::new("/nonexistent")).is_none());
    }

    #[test]
    fn hostile_text_is_bounded() {
        let l = lex();
        let long = "a?".repeat(5000);
        assert_eq!(l.restore_text(&long).chars().count(), long.chars().count());
        for s in ["?", "-", "a-", "-a", "--", "a--b", "?-?", "Ș", "ß", "ǅ", "\u{301}a", "a\u{301}"] {
            let _ = l.restore_text(s);
        }
        assert_eq!(l.restore_text("????????a"), "????????a".replace('?', "ă").replacen('ă', "î", 1));
    }
}
