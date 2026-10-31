//! Explicit bootstrap binding for standalone compiler fixtures without a sysroot.
//! Production sources obtain primitive names through builtin.prelude.

pub(crate) fn with_bool_binding(source: &str) -> String {
    use lex::TokenKind;
    let (tokens, symbols, _) = lex::lex(source, rustc_span::BytePos(0));
    let mut depth = 0usize;
    let mut needs_bool = false;
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::LBrace => depth += 1,
            TokenKind::RBrace => depth = depth.saturating_sub(1),
            TokenKind::Id
                if symbols
                    .get(&index)
                    .is_some_and(|name| name.as_str() == "bool") =>
            {
                needs_bool = true;
                if depth == 0 && index > 0 && tokens[index - 1].kind == TokenKind::Typealias {
                    return source.to_owned();
                }
            }
            _ => {}
        }
    }
    if needs_bool {
        format!("{source}\ntypealias bool = .bool'builtin;\n")
    } else {
        source.to_owned()
    }
}

#[allow(dead_code)]
pub(crate) fn write_fixture(
    path: impl AsRef<std::path::Path>,
    contents: impl AsRef<[u8]>,
) -> std::io::Result<()> {
    let path = path.as_ref();
    if path.extension().is_some_and(|extension| extension == "fl") {
        if let Ok(source) = std::str::from_utf8(contents.as_ref()) {
            return std::fs::write(path, with_bool_binding(source));
        }
    }
    std::fs::write(path, contents)
}
