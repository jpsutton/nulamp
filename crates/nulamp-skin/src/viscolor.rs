//! `viscolor.txt` visualizer palette (01 §5).

/// The 24 default visualizer colours, `0x00RRGGBB`.
pub const DEFAULT_VIS_COLORS: [u32; 24] = [
    0x000000, 0x181829, 0xEF3110, 0xCE2910, 0xD65A00, 0xD66600, 0xD67300, 0xC67B08, 0xDEA518,
    0xD6B521, 0xBDDE29, 0x94DE21, 0x29CE10, 0x32BE10, 0x39B510, 0x319C08, 0x299400, 0x188408,
    0xFFFFFF, 0xD6D6DE, 0xB5BDBD, 0xA0AAAF, 0x949CA5, 0x969696,
];

/// Parses `viscolor.txt`, starting from the default palette.
///
/// Quirks kept from the reference: a final line without a newline is ignored; a component
/// without digits stops parsing of the whole file; values wrap modulo 256.
pub fn parse(text: &str) -> [u32; 24] {
    let mut colors = DEFAULT_VIS_COLORS;
    let mut lines: Vec<&str> = text.split('\n').collect();
    // The last element is either empty (text ended with a newline) or an unterminated line.
    lines.pop();
    for (index, line) in lines.into_iter().take(24).enumerate() {
        let b = line.as_bytes();
        let mut i = 0;
        for comp in 0..3 {
            while i < b.len() && matches!(b[i], b' ' | b',' | b'\t') {
                i += 1;
            }
            let start = i;
            let mut v: u32 = 0;
            while i < b.len() && b[i].is_ascii_digit() {
                v = v.wrapping_mul(10).wrapping_add((b[i] - b'0') as u32);
                i += 1;
            }
            if i == start {
                return colors;
            }
            let shift = 16 - comp * 8;
            colors[index] = (colors[index] & !(0xFF << shift)) | ((v & 0xFF) << shift);
        }
    }
    colors
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(r: u32, g: u32, b: u32) -> String {
        format!("{r},{g},{b}, // comment\r\n")
    }

    #[test]
    fn comments_after_each_line() {
        let text: String = (0..24).map(|i| line(i, i + 1, i + 2)).collect();
        let c = parse(&text);
        for (i, &color) in c.iter().enumerate() {
            let i = i as u32;
            assert_eq!(color, (i << 16) | ((i + 1) << 8) | (i + 2));
        }
    }

    #[test]
    fn blank_line_10_stops() {
        let mut text: String = (0..9).map(|_| line(1, 2, 3)).collect();
        text.push('\n');
        text.push_str(&line(9, 9, 9));
        let c = parse(&text);
        assert!(c[..9].iter().all(|&x| x == 0x010203));
        assert_eq!(&c[9..], &DEFAULT_VIS_COLORS[9..]);
    }

    #[test]
    fn value_wraps() {
        assert_eq!(parse("300,0,0\n")[0], 44 << 16);
    }

    #[test]
    fn last_line_without_newline_ignored() {
        let c = parse("1,2,3\n4,5,6");
        assert_eq!(c[0], 0x010203);
        assert_eq!(c[1], DEFAULT_VIS_COLORS[1]);
    }

    #[test]
    fn partial_line_keeps_read_components() {
        let c = parse("10,20,x\n");
        assert_eq!(c[0], (10 << 16) | (20 << 8));
        let c = parse("-1,2,3\n");
        assert_eq!(c[0], 0);
    }
}
