//! Keys as Linux input codes, for the compositor's US keyboard layout, and
//! as the bytes an xterm sends, for programs running in the terminal pane.

const SHIFT: u32 = 42;
const CTRL: u32 = 29;
const ALT: u32 = 56;
const SUPER: u32 = 125;

/// One key press: the modifiers held and the key itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Press {
    pub modifiers: Vec<u32>,
    pub key: u32,
}

/// The press that types `c`, or `None` for a character the layout lacks.
pub fn for_char(c: char) -> Option<Press> {
    const ROWS: [(&str, &str, u32); 4] = [
        ("1234567890-=", "!@#$%^&*()_+", 2),
        ("qwertyuiop[]", "QWERTYUIOP{}", 16),
        ("asdfghjkl;'`", "ASDFGHJKL:\"~", 30),
        ("zxcvbnm,./", "ZXCVBNM<>?", 44),
    ];
    let plain = |key| {
        Some(Press {
            modifiers: vec![],
            key,
        })
    };
    let shifted = |key| {
        Some(Press {
            modifiers: vec![SHIFT],
            key,
        })
    };
    match c {
        ' ' => return plain(57),
        '\n' => return plain(28),
        '\t' => return plain(15),
        '\\' => return plain(43),
        '|' => return shifted(43),
        _ => {}
    }
    for (lower, upper, first) in ROWS {
        if let Some(i) = lower.chars().position(|l| l == c) {
            return plain(first + i as u32);
        }
        if let Some(i) = upper.chars().position(|u| u == c) {
            return shifted(first + i as u32);
        }
    }
    None
}

/// A chord such as `ctrl+shift+t`, `enter`, or `f5`.
pub fn chord(text: &str) -> Result<Press, String> {
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    let (key, mods) = parts.split_last().ok_or("an empty key")?;
    let mut modifiers = vec![];
    for m in mods {
        modifiers.push(match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => CTRL,
            "shift" => SHIFT,
            "alt" => ALT,
            "super" | "meta" => SUPER,
            other => return Err(format!("`{other}` is not a modifier in `{text}`")),
        });
    }
    let lower = key.to_ascii_lowercase();
    let code = match lower.as_str() {
        "enter" | "return" => 28,
        "escape" | "esc" => 1,
        "tab" => 15,
        "space" => 57,
        "backspace" => 14,
        "delete" | "del" => 111,
        "insert" => 110,
        "home" => 102,
        "end" => 107,
        "pageup" => 104,
        "pagedown" => 109,
        "up" => 103,
        "down" => 108,
        "left" => 105,
        "right" => 106,
        "plus" => 13,
        "minus" => 12,
        f if f.starts_with('f') && f[1..].parse::<u32>().is_ok_and(|n| (1..=12).contains(&n)) => {
            let n: u32 = f[1..].parse().unwrap();
            if n <= 10 { 58 + n } else { 76 + n }
        }
        single if single.chars().count() == 1 => {
            let press = for_char(single.chars().next().unwrap())
                .ok_or_else(|| format!("`{key}` is not on the keyboard"))?;
            // A chord names the key, so `ctrl+a` is not `ctrl+shift+a`.
            press.key
        }
        _ => return Err(format!("`{key}` is not a key name in `{text}`")),
    };
    Ok(Press {
        modifiers,
        key: code,
    })
}

/// The bytes an xterm sends for a chord, as [`chord`] names it. With
/// `app_cursor`, the program has asked for application cursor keys, which
/// change how unmodified arrows, Home, and End are sent.
pub fn terminal_bytes(text: &str, app_cursor: bool) -> Result<Vec<u8>, String> {
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    let (key, mods) = parts.split_last().ok_or("an empty key")?;
    let (mut ctrl, mut shift, mut alt) = (false, false, false);
    for m in mods {
        match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => ctrl = true,
            "shift" => shift = true,
            "alt" => alt = true,
            "super" | "meta" => {
                return Err(format!("`{m}` does not reach a program in the terminal"));
            }
            other => return Err(format!("`{other}` is not a modifier in `{text}`")),
        }
    }
    // xterm's modifier parameter for keys sent as escape sequences.
    let param = 1 + shift as u8 + 2 * alt as u8 + 4 * ctrl as u8;
    // A key sent as CSI or SS3 with a final letter, such as the arrows.
    let letter = |c: char| -> Vec<u8> {
        if param > 1 {
            format!("\x1b[1;{param}{c}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{c}").into_bytes()
        } else {
            format!("\x1b[{c}").into_bytes()
        }
    };
    // A key sent as CSI number ~, such as Delete.
    let tilde = |n: u8| -> Vec<u8> {
        if param > 1 {
            format!("\x1b[{n};{param}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };
    // F1 to F4 are SS3 letters unless modified.
    let pf = |c: char| -> Vec<u8> {
        if param > 1 {
            format!("\x1b[1;{param}{c}").into_bytes()
        } else {
            format!("\x1bO{c}").into_bytes()
        }
    };
    let lower = key.to_ascii_lowercase();
    let special = match lower.as_str() {
        "up" => Some(letter('A')),
        "down" => Some(letter('B')),
        "right" => Some(letter('C')),
        "left" => Some(letter('D')),
        "home" => Some(letter('H')),
        "end" => Some(letter('F')),
        "insert" => Some(tilde(2)),
        "delete" | "del" => Some(tilde(3)),
        "pageup" => Some(tilde(5)),
        "pagedown" => Some(tilde(6)),
        "f1" => Some(pf('P')),
        "f2" => Some(pf('Q')),
        "f3" => Some(pf('R')),
        "f4" => Some(pf('S')),
        f if f.starts_with('f') && f[1..].parse::<u8>().is_ok_and(|n| (5..=12).contains(&n)) => {
            const CODES: [u8; 8] = [15, 17, 18, 19, 20, 21, 23, 24];
            Some(tilde(CODES[f[1..].parse::<usize>().unwrap() - 5]))
        }
        _ => None,
    };
    if let Some(bytes) = special {
        return Ok(bytes);
    }
    let mut bytes = match lower.as_str() {
        "enter" | "return" => vec![b'\r'],
        "escape" | "esc" => vec![0x1b],
        "tab" if shift => return Ok(b"\x1b[Z".to_vec()),
        "tab" => vec![b'\t'],
        "space" if ctrl => vec![0],
        "space" => vec![b' '],
        "backspace" if ctrl => vec![0x08],
        "backspace" => vec![0x7f],
        "plus" => vec![b'+'],
        "minus" => vec![b'-'],
        single if single.chars().count() == 1 => {
            let c = key.chars().next().unwrap();
            if for_char(c).is_none() {
                return Err(format!("`{key}` is not on the keyboard"));
            }
            if ctrl {
                // Ctrl clears the top bits of @, the letters, and [\]^_.
                match c.to_ascii_uppercase() {
                    u @ '@'..='_' => vec![u as u8 & 0x1f],
                    '?' => vec![0x7f],
                    _ => return Err(format!("ctrl+{key} sends nothing to a terminal")),
                }
            } else if shift {
                vec![c.to_ascii_uppercase() as u8]
            } else {
                vec![c as u8]
            }
        }
        _ => return Err(format!("`{key}` is not a key name in `{text}`")),
    };
    if alt {
        bytes.insert(0, 0x1b);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn characters() {
        assert_eq!(
            for_char('7'),
            Some(Press {
                modifiers: vec![],
                key: 8
            })
        );
        assert_eq!(
            for_char('a'),
            Some(Press {
                modifiers: vec![],
                key: 30
            })
        );
        assert_eq!(
            for_char('A'),
            Some(Press {
                modifiers: vec![SHIFT],
                key: 30
            })
        );
        assert_eq!(
            for_char('*'),
            Some(Press {
                modifiers: vec![SHIFT],
                key: 9
            })
        );
        assert_eq!(
            for_char('+'),
            Some(Press {
                modifiers: vec![SHIFT],
                key: 13
            })
        );
        assert_eq!(
            for_char('?'),
            Some(Press {
                modifiers: vec![SHIFT],
                key: 53
            })
        );
        assert_eq!(for_char('é'), None);
    }

    #[test]
    fn chords() {
        assert_eq!(chord("enter").unwrap().key, 28);
        assert_eq!(
            chord("ctrl+k").unwrap(),
            Press {
                modifiers: vec![CTRL],
                key: 37
            }
        );
        assert_eq!(chord("ctrl+shift+T").unwrap().modifiers, vec![CTRL, SHIFT]);
        assert_eq!(chord("f5").unwrap().key, 63);
        assert_eq!(chord("f12").unwrap().key, 88);
        assert!(chord("hyper+a").is_err());
        assert!(chord("ctrl+nope").is_err());
    }

    #[test]
    fn terminal_keys() {
        let bytes = |text| terminal_bytes(text, false).unwrap();
        assert_eq!(bytes("enter"), b"\r");
        assert_eq!(bytes("ctrl+c"), [3]);
        assert_eq!(bytes("ctrl+C"), [3]);
        assert_eq!(bytes("ctrl+["), [0x1b]);
        assert_eq!(bytes("q"), b"q");
        assert_eq!(bytes("shift+q"), b"Q");
        assert_eq!(bytes("alt+f"), b"\x1bf");
        assert_eq!(bytes("up"), b"\x1b[A");
        assert_eq!(terminal_bytes("up", true).unwrap(), b"\x1bOA");
        assert_eq!(bytes("ctrl+right"), b"\x1b[1;5C");
        assert_eq!(bytes("shift+tab"), b"\x1b[Z");
        assert_eq!(bytes("delete"), b"\x1b[3~");
        assert_eq!(bytes("f1"), b"\x1bOP");
        assert_eq!(bytes("f5"), b"\x1b[15~");
        assert_eq!(bytes("f12"), b"\x1b[24~");
        assert!(terminal_bytes("super+a", false).is_err());
        assert!(terminal_bytes("ctrl+1", false).is_err());
        assert!(terminal_bytes("nope", false).is_err());
    }
}
