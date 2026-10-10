//! Random format strings must never panic.

use std::collections::HashMap;

use nulamp_tf::{Formatter, MAX_LEN, SysTime};

const NOW: SysTime = SysTime {
    year: 2026,
    month: 1,
    day: 1,
    hour: 0,
    minute: 0,
    second: 0,
};

const PIECES: &[&str] = &[
    "%",
    "$",
    "[",
    "]",
    "(",
    ")",
    "'",
    ",",
    " ",
    "a",
    "Z",
    "0",
    "-1",
    "9999999999",
    "é",
    "ｘ",
    "\n",
    "%artist%",
    "%album%",
    "%filename%",
    "%track%",
    "$if(",
    "$if2(",
    "$if3(",
    "$decode(",
    "$select(",
    "$iflonger(",
    "$ifgreater(",
    "$ifstrequal(",
    "$len(",
    "$left(",
    "$right(",
    "$pad(",
    "$lpad(",
    "$padcut(",
    "$repeat(",
    "$caps(",
    "$abbr(",
    "$strrchr(",
    "$strstr(",
    "$substr(",
    "$split(",
    "$replace(",
    "$char(",
    "$tab(",
    "$num(",
    "$hex(",
    "$add(",
    "$div(",
    "$mod(",
    "$muldiv(",
    "$min(",
    "$directory(",
    "$pathltrim(",
    "$pathrpart(",
    "$pathsafe(",
    "$put(",
    "$puts(",
    "$get(",
    "$null(",
    "$systime_second(",
    "$foo(",
    "-2147483648",
    "2147483647",
    "''",
    "%%",
    "$$",
];

/// xorshift64*.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn fields() -> HashMap<String, String> {
    [
        ("artist", "Daft Punk"),
        ("track", "3/14"),
        ("filename", "/m/a\\b.mp3"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

#[test]
fn random_strings_do_not_panic() {
    let fields = fields();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..20_000 {
        let mut s = String::new();
        for _ in 0..rng.below(40) {
            s.push_str(PIECES[rng.below(PIECES.len())]);
        }
        let out = Formatter::new(&s).format_at(&fields, NOW);
        assert!(out.len() <= MAX_LEN, "{s}");
        assert!(!out.contains(['\r', '\n']), "{s}");
    }
}

#[test]
fn random_bytes_do_not_panic() {
    let fields = fields();
    let mut rng = Rng(42);
    for _ in 0..5_000 {
        let bytes: Vec<u8> = (0..rng.below(64)).map(|_| rng.below(128) as u8).collect();
        let s = String::from_utf8_lossy(&bytes);
        Formatter::new(&s).format_at(&fields, NOW);
    }
}

#[test]
fn deep_nesting_does_not_panic() {
    let fields = fields();
    for open in ["$upper(", "[", "$if(%artist%,", "'"] {
        for n in [127, 128, 129, 10_000, 200_000] {
            let s = format!("{}x{}", open.repeat(n), ")".repeat(n));
            Formatter::new(&s).format_at(&fields, NOW);
            let s = format!("{}x{}", open.repeat(n), "]".repeat(n));
            Formatter::new(&s).format_at(&fields, NOW);
        }
    }
}
