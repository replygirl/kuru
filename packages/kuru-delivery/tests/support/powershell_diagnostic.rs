// Stock PS5.1 can serialize redirected errors as CLIXML even when its output
// format is Text. Decode only that explicit envelope and Error stream; progress
// or warning text must never satisfy a rejection assertion. Keep malformed or
// ordinary stderr unchanged so a diagnostic failure remains visible.
pub fn message(bytes: &[u8]) -> String {
    let raw = String::from_utf8_lossy(bytes);
    let errors = raw
        .strip_prefix("#< CLIXML")
        .and_then(|xml| roxmltree::Document::parse(xml.trim_start()).ok())
        .map(|document| {
            document
                .descendants()
                .filter(|node| node.has_tag_name("S") && node.attribute("S") == Some("Error"))
                .filter_map(|node| node.text())
                .map(unescape)
                .collect::<Vec<_>>()
                .join("\n")
        });
    errors
        .as_deref()
        .unwrap_or(&raw)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn unescape(text: &str) -> String {
    let mut units = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(encoded) = rest.strip_prefix("_x")
            && let Some(digits) = encoded.get(..4)
            && encoded.as_bytes().get(4) == Some(&b'_')
            && let Ok(unit) = u16::from_str_radix(digits, 16)
        {
            units.push(unit);
            rest = &encoded[5..];
        } else {
            let character = rest.chars().next().unwrap();
            units.extend(character.encode_utf16(&mut [0; 2]).iter().copied());
            rest = &rest[character.len_utf8()..];
        }
    }
    String::from_utf16_lossy(&units)
}
