//! Keys as Linux input codes, for the compositor's US keyboard layout.

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
}
