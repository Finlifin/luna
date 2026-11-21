//! `@use` macro import parsing, diamond deduplication, shadowing, visibility, and lookup.
use crate::registry::{ExpansionContext, MacroDescriptor, MacroId};
use crate::{MacroError, MacroErrorKind};
use rustc_span::{BytePos, Span};
use std::collections::HashMap;

#[derive(Clone, Debug)]
enum ExplicitBinding {
    Macro(MacroId, Span),
    Module(Vec<String>, Span),
}

#[derive(Clone, Debug)]
enum GlobMacroBinding {
    Unique(MacroId),
    Ambiguous(Vec<String>),
}

#[derive(Clone, Debug)]
enum GlobModuleBinding {
    Unique(Vec<String>),
    Ambiguous(Vec<String>),
}

/// File-local macro import scope built sequentially from top-to-bottom `@use` directives.
#[derive(Clone, Debug, Default)]
pub struct MacroScope {
    explicit: HashMap<String, ExplicitBinding>,
    glob_macros: HashMap<String, GlobMacroBinding>,
    glob_modules: HashMap<String, GlobModuleBinding>,
}

impl MacroScope {
    pub fn new() -> Self {
        Self::default()
    }

    fn bind_explicit_macro(
        &mut self,
        name: String,
        id: MacroId,
        span: Span,
    ) -> Result<(), MacroError> {
        if let Some(existing) = self.explicit.get(&name) {
            match existing {
                ExplicitBinding::Macro(prev_id, _) if *prev_id == id => return Ok(()),
                ExplicitBinding::Macro(_, prev_span) => {
                    return Err(MacroError::new(
                        MacroErrorKind::AmbiguousMacro,
                        format!("macro `{name}` is already explicitly imported with a different identity"),
                        span,
                    )
                    .with_label(*prev_span, "previous explicit macro import here"));
                }
                ExplicitBinding::Module(_, prev_span) => {
                    return Err(MacroError::new(
                        MacroErrorKind::AmbiguousMacro,
                        format!("macro `{name}` conflicts with an explicit macro module import of the same name"),
                        span,
                    )
                    .with_label(*prev_span, "previous module import here"));
                }
            }
        }
        self.explicit.insert(name, ExplicitBinding::Macro(id, span));
        Ok(())
    }

    fn bind_explicit_module(
        &mut self,
        name: String,
        path: Vec<String>,
        span: Span,
    ) -> Result<(), MacroError> {
        if let Some(existing) = self.explicit.get(&name) {
            match existing {
                ExplicitBinding::Module(prev_path, _) if *prev_path == path => return Ok(()),
                ExplicitBinding::Module(_, prev_span) => {
                    return Err(MacroError::new(
                        MacroErrorKind::AmbiguousMacro,
                        format!("macro module `{name}` conflicts with an explicit import to a different module"),
                        span,
                    )
                    .with_label(*prev_span, "previous module import here"));
                }
                ExplicitBinding::Macro(_, prev_span) => {
                    return Err(MacroError::new(
                        MacroErrorKind::AmbiguousMacro,
                        format!("macro module `{name}` conflicts with an explicit macro import of the same name"),
                        span,
                    )
                    .with_label(*prev_span, "previous macro import here"));
                }
            }
        }
        self.explicit
            .insert(name, ExplicitBinding::Module(path, span));
        Ok(())
    }

    fn add_glob_macro(&mut self, name: String, id: MacroId, source_path: String) {
        match self.glob_macros.get_mut(&name) {
            None => {
                self.glob_macros.insert(name, GlobMacroBinding::Unique(id));
            }
            Some(GlobMacroBinding::Unique(prev_id)) if *prev_id == id => {}
            Some(GlobMacroBinding::Unique(_)) => {
                self.glob_macros
                    .insert(name, GlobMacroBinding::Ambiguous(vec![source_path]));
            }
            Some(GlobMacroBinding::Ambiguous(sources)) => {
                if !sources.contains(&source_path) {
                    sources.push(source_path);
                }
            }
        }
    }

    fn add_glob_module(&mut self, name: String, path: Vec<String>, source_path: String) {
        match self.glob_modules.get_mut(&name) {
            None => {
                self.glob_modules
                    .insert(name, GlobModuleBinding::Unique(path));
            }
            Some(GlobModuleBinding::Unique(prev_path)) if *prev_path == path => {}
            Some(GlobModuleBinding::Unique(_)) => {
                self.glob_modules
                    .insert(name, GlobModuleBinding::Ambiguous(vec![source_path]));
            }
            Some(GlobModuleBinding::Ambiguous(sources)) => {
                if !sources.contains(&source_path) {
                    sources.push(source_path);
                }
            }
        }
    }

    /// Resolve `@path` at a macro call site against imported bindings and direct package roots.
    pub fn resolve_call_path<'a>(
        &self,
        path: &[String],
        span: Span,
        ctx: &ExpansionContext<'a>,
    ) -> Result<&'a MacroDescriptor, MacroError> {
        let joined = path.join(".");
        let Some((first, rest)) = path.split_first() else {
            return Err(MacroError::new(
                MacroErrorKind::UndefinedMacro,
                "empty macro path after `@`",
                span,
            ));
        };

        if rest.is_empty() {
            if let Some(binding) = self.explicit.get(first) {
                return match binding {
                    ExplicitBinding::Macro(id, _) => {
                        let desc = ctx.registry.get(*id).ok_or_else(|| {
                            MacroError::new(
                                MacroErrorKind::UndefinedMacro,
                                format!("imported macro `@{first}` is missing from registry"),
                                span,
                            )
                        })?;
                        if !desc.is_accessible_from(ctx.current_package, &ctx.current_module_path) {
                            return Err(MacroError::new(
                                MacroErrorKind::MacroVisibilityError,
                                format!(
                                    "macro `@{}` is not accessible from this module",
                                    desc.full_path()
                                ),
                                span,
                            ));
                        }
                        Ok(desc)
                    }
                    ExplicitBinding::Module(mod_path, _) => Err(MacroError::new(
                        MacroErrorKind::UndefinedMacro,
                        format!(
                            "`@{first}` names macro module `{}`, not a macro",
                            mod_path.join(".")
                        ),
                        span,
                    )),
                };
            }

            if let Some(glob) = self.glob_macros.get(first) {
                return match glob {
                    GlobMacroBinding::Unique(id) => {
                        let desc = ctx.registry.get(*id).ok_or_else(|| {
                            MacroError::new(
                                MacroErrorKind::UndefinedMacro,
                                format!("imported macro `@{first}` is missing from registry"),
                                span,
                            )
                        })?;
                        if !desc.is_accessible_from(ctx.current_package, &ctx.current_module_path) {
                            return Err(MacroError::new(
                                MacroErrorKind::MacroVisibilityError,
                                format!(
                                    "macro `@{}` is not accessible from this module",
                                    desc.full_path()
                                ),
                                span,
                            ));
                        }
                        Ok(desc)
                    }
                    GlobMacroBinding::Ambiguous(candidates) => Err(MacroError::new(
                        MacroErrorKind::AmbiguousMacro,
                        format!(
                            "glob-imported macro `@{first}` is ambiguous (candidates include {}); disambiguate with an explicit `@use`",
                            candidates.join(", ")
                        ),
                        span,
                    )),
                };
            }

            return Err(MacroError::new(
                MacroErrorKind::UndefinedMacro,
                format!(
                    "undefined macro `@{joined}` (import it with `@use ...` or use a qualified `@package.path`)"
                ),
                span,
            ));
        }

        // Multi-segment `@first.rest...`: check module aliases first, then full root paths.
        let mut resolved_full: Option<Vec<String>> = None;
        if let Some(ExplicitBinding::Module(mod_prefix, _)) = self.explicit.get(first) {
            let mut candidate = mod_prefix.clone();
            candidate.extend(rest.iter().cloned());
            resolved_full = Some(candidate);
        } else if let Some(glob_mod) = self.glob_modules.get(first) {
            match glob_mod {
                GlobModuleBinding::Unique(mod_prefix) if self.explicit.get(first).is_none() => {
                    let mut candidate = mod_prefix.clone();
                    candidate.extend(rest.iter().cloned());
                    resolved_full = Some(candidate);
                }
                GlobModuleBinding::Ambiguous(sources) if self.explicit.get(first).is_none() => {
                    return Err(MacroError::new(
                        MacroErrorKind::AmbiguousMacro,
                        format!(
                            "glob-imported macro module `{first}` is ambiguous ({})",
                            sources.join(", ")
                        ),
                        span,
                    ));
                }
                _ => {}
            }
        }

        if let Some(aliased) = resolved_full {
            if let Some(desc) = ctx.registry.lookup_path(&aliased) {
                if !desc.is_accessible_from(ctx.current_package, &ctx.current_module_path) {
                    return Err(MacroError::new(
                        MacroErrorKind::MacroVisibilityError,
                        format!(
                            "macro `@{}` is not accessible from this module",
                            desc.full_path()
                        ),
                        span,
                    ));
                }
                return Ok(desc);
            }
        }

        if let Some(desc) = ctx.registry.lookup_path(path) {
            if !desc.is_accessible_from(ctx.current_package, &ctx.current_module_path) {
                return Err(MacroError::new(
                    MacroErrorKind::MacroVisibilityError,
                    format!(
                        "macro `@{}` is not accessible from this module",
                        desc.full_path()
                    ),
                    span,
                ));
            }
            return Ok(desc);
        }

        Err(MacroError::new(
            MacroErrorKind::UndefinedMacro,
            format!("undefined macro `@{joined}`"),
            span,
        ))
    }
}

pub(crate) fn is_ident_start(c: char) -> bool {
    c.is_alphabetic()
        || c == '_'
        || matches!(
            c,
            '\u{1F000}'..='\u{1F9FF}' | '\u{2600}'..='\u{26FF}' | '\u{2700}'..='\u{27BF}'
        )
}

pub(crate) fn is_ident_continue(c: char) -> bool {
    c.is_alphanumeric()
        || c == '_'
        || matches!(
            c,
            '\u{1F000}'..='\u{1F9FF}' | '\u{2600}'..='\u{26FF}' | '\u{2700}'..='\u{27BF}'
        )
}

pub(crate) fn skip_ws_and_comments(
    src: &str,
    mut pos: usize,
    base_pos: BytePos,
) -> Result<usize, MacroError> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    while pos < len {
        let ch = src[pos..].chars().next().unwrap();
        if ch.is_whitespace() {
            pos += ch.len_utf8();
            continue;
        }
        if bytes[pos] == b'-' && bytes.get(pos + 1) == Some(&b'-') {
            pos += 2;
            while pos < len && bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        if bytes[pos] == b'{' && bytes.get(pos + 1) == Some(&b'-') {
            let start = pos;
            pos += 2;
            let mut depth = 1usize;
            while pos < len && depth > 0 {
                if bytes[pos] == b'{' && bytes.get(pos + 1) == Some(&b'-') {
                    depth += 1;
                    pos += 2;
                } else if bytes[pos] == b'-' && bytes.get(pos + 1) == Some(&b'}') {
                    depth -= 1;
                    pos += 2;
                } else {
                    pos += 1;
                }
            }
            if depth > 0 {
                return Err(MacroError::new(
                    MacroErrorKind::InvalidMacroImport,
                    "unterminated block comment in `@use` or macro header",
                    Span::new(
                        base_pos + BytePos(start as u32),
                        base_pos + BytePos(len as u32),
                    ),
                ));
            }
            continue;
        }
        break;
    }
    Ok(pos)
}

pub(crate) fn scan_ident_at(src: &str, pos: usize) -> Option<(String, usize)> {
    let tail = src.get(pos..)?;
    let mut chars = tail.char_indices();
    let (_, first) = chars.next()?;
    if !is_ident_start(first) {
        return None;
    }
    let mut end = first.len_utf8();
    for (idx, ch) in chars {
        if is_ident_continue(ch) {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    Some((tail[..end].to_string(), pos + end))
}

fn span_range(base_pos: BytePos, start: usize, end: usize) -> Span {
    Span::new(
        base_pos + BytePos(start as u32),
        base_pos + BytePos(end as u32),
    )
}

fn apply_glob(
    prefix: &[String],
    span: Span,
    scope: &mut MacroScope,
    ctx: &ExpansionContext<'_>,
) -> Result<(), MacroError> {
    if !ctx.registry.module_exists(prefix) {
        return Err(MacroError::new(
            MacroErrorKind::InvalidMacroImport,
            format!(
                "cannot glob-import unknown macro module `{}`",
                prefix.join(".")
            ),
            span,
        ));
    }
    for (name, macro_id_opt, full_path) in ctx.registry.glob_members(prefix) {
        if let Some(macro_id) = macro_id_opt {
            if let Some(desc) = ctx.registry.get(macro_id) {
                if desc.is_accessible_from(ctx.current_package, &ctx.current_module_path) {
                    scope.add_glob_macro(name, macro_id, full_path.join("."));
                }
            }
        } else {
            scope.add_glob_module(name, full_path.clone(), full_path.join("."));
        }
    }
    Ok(())
}

fn apply_leaf(
    path: &[String],
    alias: Option<String>,
    span: Span,
    scope: &mut MacroScope,
    ctx: &ExpansionContext<'_>,
) -> Result<(), MacroError> {
    if let Some(desc) = ctx.registry.lookup_path(path) {
        if !desc.is_accessible_from(ctx.current_package, &ctx.current_module_path) {
            return Err(MacroError::new(
                MacroErrorKind::MacroVisibilityError,
                format!(
                    "macro `{}` is not accessible from this module",
                    desc.full_path()
                ),
                span,
            ));
        }
        let local_name = alias.unwrap_or_else(|| path.last().cloned().unwrap_or_default());
        return scope.bind_explicit_macro(local_name, desc.id, span);
    }
    if ctx.registry.module_exists(path) {
        let local_name = alias.unwrap_or_else(|| path.last().cloned().unwrap_or_default());
        return scope.bind_explicit_module(local_name, path.to_vec(), span);
    }
    Err(MacroError::new(
        MacroErrorKind::InvalidMacroImport,
        format!("cannot resolve `@use` import path `{}`", path.join(".")),
        span,
    ))
}

fn parse_import_branch(
    src: &str,
    pos: &mut usize,
    base_pos: BytePos,
    prefix: &[String],
    scope: &mut MacroScope,
    ctx: &ExpansionContext<'_>,
    allow_bare_glob: bool,
) -> Result<bool, MacroError> {
    let bytes = src.as_bytes();
    *pos = skip_ws_and_comments(src, *pos, base_pos)?;
    let start = *pos;

    if allow_bare_glob && bytes.get(*pos) == Some(&b'*') {
        *pos += 1;
        apply_glob(prefix, span_range(base_pos, start, *pos), scope, ctx)?;
        return Ok(false);
    }

    let Some((first_seg, after_first)) = scan_ident_at(src, *pos) else {
        return Err(MacroError::new(
            MacroErrorKind::InvalidMacroImport,
            "expected macro path segment in `@use` statement",
            span_range(base_pos, *pos, (*pos + 1).min(bytes.len())),
        ));
    };
    *pos = after_first;
    let mut full_path = prefix.to_vec();
    full_path.push(first_seg);

    loop {
        let after_ws = skip_ws_and_comments(src, *pos, base_pos)?;
        if bytes.get(after_ws) != Some(&b'.') {
            *pos = after_ws;
            break;
        }
        let after_dot = skip_ws_and_comments(src, after_ws + 1, base_pos)?;
        match bytes.get(after_dot) {
            Some(&b'*') => {
                *pos = after_dot + 1;
                apply_glob(&full_path, span_range(base_pos, start, *pos), scope, ctx)?;
                return Ok(false);
            }
            Some(&b'{') => {
                *pos = after_dot + 1;
                loop {
                    *pos = skip_ws_and_comments(src, *pos, base_pos)?;
                    if bytes.get(*pos) == Some(&b'}') {
                        *pos += 1;
                        return Ok(true);
                    }
                    let _ = parse_import_branch(src, pos, base_pos, &full_path, scope, ctx, true)?;
                    *pos = skip_ws_and_comments(src, *pos, base_pos)?;
                    if bytes.get(*pos) == Some(&b',') {
                        *pos += 1;
                        continue;
                    }
                    if bytes.get(*pos) == Some(&b'}') {
                        *pos += 1;
                        return Ok(true);
                    }
                    return Err(MacroError::new(
                        MacroErrorKind::InvalidMacroImport,
                        "expected `,` or `}` inside `@use` group",
                        span_range(base_pos, *pos, (*pos + 1).min(bytes.len())),
                    ));
                }
            }
            _ => {
                let Some((seg, next_end)) = scan_ident_at(src, after_dot) else {
                    return Err(MacroError::new(
                        MacroErrorKind::InvalidMacroImport,
                        "expected identifier, `*`, or `{` after `.` in `@use` path",
                        span_range(base_pos, after_dot, (after_dot + 1).min(bytes.len())),
                    ));
                };
                full_path.push(seg);
                *pos = next_end;
            }
        }
    }

    let mut alias = None;
    let check_as = skip_ws_and_comments(src, *pos, base_pos)?;
    if let Some((kw, after_as)) = scan_ident_at(src, check_as) {
        if kw == "as" {
            let alias_pos = skip_ws_and_comments(src, after_as, base_pos)?;
            let Some((alias_name, after_alias)) = scan_ident_at(src, alias_pos) else {
                return Err(MacroError::new(
                    MacroErrorKind::InvalidMacroImport,
                    "expected alias identifier after `as` in `@use` statement",
                    span_range(base_pos, alias_pos, (alias_pos + 1).min(bytes.len())),
                ));
            };
            alias = Some(alias_name);
            *pos = after_alias;
        }
    }

    apply_leaf(
        &full_path,
        alias,
        span_range(base_pos, start, *pos),
        scope,
        ctx,
    )?;
    Ok(false)
}

/// Parse an `@use ...;` macro import statement starting at `at_pos` (`src[at_pos..]` starts with `@use`).
/// Returns the byte offset immediately after the consumed statement.
pub fn parse_at_use(
    src: &str,
    at_pos: usize,
    base_pos: BytePos,
    scope: &mut MacroScope,
    ctx: &ExpansionContext<'_>,
) -> Result<usize, MacroError> {
    let bytes = src.as_bytes();
    let mut pos = at_pos + 4; // skip "@use"
    let ended_with_rbrace = parse_import_branch(src, &mut pos, base_pos, &[], scope, ctx, false)?;
    pos = skip_ws_and_comments(src, pos, base_pos)?;
    if bytes.get(pos) == Some(&b';') {
        pos += 1;
    } else if !ended_with_rbrace {
        return Err(MacroError::new(
            MacroErrorKind::InvalidMacroImport,
            "expected `;` after `@use` import path",
            span_range(base_pos, at_pos, pos),
        ));
    }
    Ok(pos)
}
