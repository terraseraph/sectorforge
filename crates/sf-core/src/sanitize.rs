//! Turning names read off a damaged disk into safe path components. A
//! corrupted or malicious MFT can contain `..`, slashes, control characters
//! or names Windows refuses (`CON`, `aux.txt`, trailing dots). Every name is
//! cleaned the same way on every OS, so a recovery made on Linux can still be
//! copied onto a Windows machine.

const MAX_COMPONENT_UTF16: usize = 240;

/// A single, non-empty path component containing no separators, no `.`/`..`,
/// and nothing Windows or Linux would reject or reinterpret.
pub fn safe_component(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 || c == '\u{7F}' => '_',
            c => c,
        })
        .collect();

    // Windows silently strips trailing dots and spaces, which would make two
    // different names collide (or turn ".." into "").
    let keep = s.trim_end_matches(['.', ' ']).len();
    let tail = s[keep..].chars().count();
    s.truncate(keep);
    s.push_str(&"_".repeat(tail));

    if s.is_empty() {
        s.push('_');
    }
    if is_reserved_windows_name(&s) {
        s.insert(0, '_');
    }
    truncate_utf16(&mut s, MAX_COMPONENT_UTF16);
    s
}

fn is_reserved_windows_name(s: &str) -> bool {
    let stem = s.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.chars().count() == 4
            && matches!(stem.chars().nth(3), Some('0'..='9' | '¹' | '²' | '³')))
}

/// Shortens `s` to at most `max` UTF-16 units, keeping the extension when
/// there is a short one.
fn truncate_utf16(s: &mut String, max: usize) {
    if s.encode_utf16().count() <= max {
        return;
    }
    let ext = s
        .rfind('.')
        .map(|i| s[i..].to_string())
        .filter(|e| e.encode_utf16().count() <= 16)
        .unwrap_or_default();
    let mut stem = s[..s.len() - ext.len()].to_string();
    while stem.encode_utf16().count() + ext.encode_utf16().count() > max {
        stem.pop();
    }
    *s = stem + &ext;
}

/// `name` with ` (id N)` before its extension, to keep two different files
/// with the same name (e.g. a deleted and a live `report.docx`) apart.
pub fn with_id_suffix(name: &str, id: u64) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => format!("{} (id {id}){}", &name[..i], &name[i..]),
        _ => format!("{name} (id {id})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_traversal_and_separators() {
        assert_eq!(safe_component(".."), "__");
        assert_eq!(safe_component("."), "_");
        assert_eq!(safe_component("a/b\\c"), "a_b_c");
        assert_eq!(safe_component("..\\..\\Windows"), ".._.._Windows");
        assert!(!safe_component("../../etc/passwd").contains('/'));
        assert_eq!(safe_component(""), "_");
    }

    #[test]
    fn blocks_reserved_and_invalid_names() {
        assert_eq!(safe_component("CON"), "_CON");
        assert_eq!(safe_component("aux.txt"), "_aux.txt");
        assert_eq!(safe_component("com1.log"), "_com1.log");
        assert_eq!(safe_component("COM10"), "COM10");
        assert_eq!(safe_component("lpt9"), "_lpt9");
        assert_eq!(safe_component("a:b*c?.txt"), "a_b_c_.txt");
        assert_eq!(safe_component("tab\there"), "tab_here");
        assert_eq!(safe_component("name. "), "name__");
        assert_eq!(safe_component("café ünïcode.txt"), "café ünïcode.txt");
    }

    #[test]
    fn truncates_long_names_keeping_extension() {
        let long = "x".repeat(400) + ".jpg";
        let s = safe_component(&long);
        assert_eq!(s.encode_utf16().count(), MAX_COMPONENT_UTF16);
        assert!(s.ends_with(".jpg"));
    }

    #[test]
    fn id_suffix_goes_before_extension() {
        assert_eq!(with_id_suffix("report.docx", 42), "report (id 42).docx");
        assert_eq!(with_id_suffix("README", 7), "README (id 7)");
        assert_eq!(with_id_suffix(".hidden", 7), ".hidden (id 7)");
    }
}
