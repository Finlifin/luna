//! Configuration model, `flurryfmt.fl` parser, CLI override parser, and `.ignore` matching.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use ast::{Ast, NodeIndex, NodeKind};
use parse::parser::Parser;
use rustc_span::{FileName, SourceMap, source_map::FilePathMapping};

pub const CONFIG_FILE_NAME: &str = "flurryfmt.fl";

/// Line-ending policy for formatted output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewlineStyle {
    Lf,
    CrLf,
    Auto,
}

impl NewlineStyle {
    pub fn as_symbol_str(self) -> &'static str {
        match self {
            Self::Lf => ".lf",
            Self::CrLf => ".crlf",
            Self::Auto => ".auto",
        }
    }

    pub fn parse_value(raw: &str) -> Option<Self> {
        let trimmed = raw.trim().strip_prefix('.').unwrap_or(raw.trim());
        match trimmed {
            "lf" | "Lf" | "LF" | "unix" | "Unix" => Some(Self::Lf),
            "crlf" | "CrLf" | "CRLF" | "windows" | "Windows" => Some(Self::CrLf),
            "auto" | "Auto" | "native" | "Native" => Some(Self::Auto),
            _ => None,
        }
    }

    /// Resolve the concrete line separator to emit for a given input buffer.
    pub fn resolve(self, original_source: &str) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::Auto => {
                if original_source.contains("\r\n") {
                    "\r\n"
                } else {
                    "\n"
                }
            }
        }
    }
}

/// Sub-element single-line width heuristics mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmallHeuristics {
    Default,
    Off,
    Max,
}

impl SmallHeuristics {
    pub fn as_symbol_str(self) -> &'static str {
        match self {
            Self::Default => ".default",
            Self::Off => ".off",
            Self::Max => ".max",
        }
    }

    pub fn parse_value(raw: &str) -> Option<Self> {
        let trimmed = raw.trim().strip_prefix('.').unwrap_or(raw.trim());
        match trimmed {
            "default" | "Default" => Some(Self::Default),
            "off" | "Off" => Some(Self::Off),
            "max" | "Max" => Some(Self::Max),
            _ => None,
        }
    }
}

/// Concrete sub-budgets derived from `max_width` and `use_small_heuristics`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WidthHeuristics {
    pub fn_call_width: usize,
    pub struct_lit_width: usize,
    pub struct_variant_width: usize,
    pub array_width: usize,
    pub chain_width: usize,
    pub single_line_if_else_max_width: usize,
    pub single_line_let_else_max_width: usize,
}

impl WidthHeuristics {
    pub fn from_config(max_width: usize, mode: SmallHeuristics) -> Self {
        match mode {
            SmallHeuristics::Off => Self {
                fn_call_width: 0,
                struct_lit_width: 0,
                struct_variant_width: 0,
                array_width: 0,
                chain_width: 0,
                single_line_if_else_max_width: 0,
                single_line_let_else_max_width: 0,
            },
            SmallHeuristics::Max => Self {
                fn_call_width: max_width,
                struct_lit_width: max_width,
                struct_variant_width: max_width,
                array_width: max_width,
                chain_width: max_width,
                single_line_if_else_max_width: max_width,
                single_line_let_else_max_width: max_width,
            },
            SmallHeuristics::Default => Self {
                fn_call_width: (max_width * 60 / 100).max(1),
                struct_lit_width: (max_width * 35 / 100).max(1),
                struct_variant_width: (max_width * 35 / 100).max(1),
                array_width: (max_width * 60 / 100).max(1),
                chain_width: (max_width * 60 / 100).max(1),
                single_line_if_else_max_width: (max_width * 50 / 100).max(1),
                single_line_let_else_max_width: (max_width * 50 / 100).max(1),
            },
        }
    }
}

/// Configuration error (invalid `flurryfmt.fl` or `--config` / `--config-path`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub message: String,
}

impl ConfigError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ConfigError {}

/// Complete Flurry formatter configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatConfig {
    pub max_width: usize,
    pub tab_spaces: usize,
    pub newline_style: NewlineStyle,
    pub use_small_heuristics: SmallHeuristics,
    pub reorder_imports: bool,
    pub normalize_comments: bool,
    pub ignore: Vec<String>,
    /// Base directory of the loaded `flurryfmt.fl` (if any), used for `.ignore` resolution.
    pub config_dir: Option<PathBuf>,
}

impl Default for FormatConfig {
    fn default() -> Self {
        Self {
            max_width: 100,
            tab_spaces: 4,
            newline_style: NewlineStyle::Lf,
            use_small_heuristics: SmallHeuristics::Default,
            reorder_imports: true,
            normalize_comments: true,
            ignore: Vec::new(),
            config_dir: None,
        }
    }
}

impl FormatConfig {
    pub fn width_heuristics(&self) -> WidthHeuristics {
        WidthHeuristics::from_config(self.max_width, self.use_small_heuristics)
    }

    /// Discover `flurryfmt.fl` starting from `start_dir` (or load `explicit_config_path`).
    pub fn discover(
        start_dir: &Path,
        explicit_config_path: Option<&Path>,
    ) -> Result<Self, ConfigError> {
        let mut config = Self::default();
        if let Some(explicit) = explicit_config_path {
            let file_path = if explicit.is_dir() {
                let candidate = explicit.join(CONFIG_FILE_NAME);
                if !candidate.is_file() {
                    return Err(ConfigError::new(format!(
                        "unable to find `{CONFIG_FILE_NAME}` in `{}`",
                        explicit.display()
                    )));
                }
                candidate
            } else {
                if !explicit.is_file() {
                    return Err(ConfigError::new(format!(
                        "configuration file `{}` does not exist",
                        explicit.display()
                    )));
                }
                explicit.to_path_buf()
            };
            config.load_file(&file_path)?;
            return Ok(config);
        }

        let mut cursor = Some(start_dir);
        while let Some(dir) = cursor {
            let candidate = dir.join(CONFIG_FILE_NAME);
            if candidate.is_file() {
                config.load_file(&candidate)?;
                return Ok(config);
            }
            cursor = dir.parent();
        }

        Ok(config)
    }

    /// Load and apply a `flurryfmt.fl` file.
    pub fn load_file(&mut self, path: &Path) -> Result<(), ConfigError> {
        let content = fs::read_to_string(path).map_err(|err| {
            ConfigError::new(format!("failed to read `{}`: {err}", path.display()))
        })?;
        self.parse_flurry_config(&content, &path.display().to_string())?;
        self.config_dir = path.parent().map(Path::to_path_buf);
        Ok(())
    }

    /// Parse `flurryfmt.fl` top-level Flurry `Configuration` items.
    pub fn parse_flurry_config(&mut self, src: &str, origin: &str) -> Result<(), ConfigError> {
        if src.trim().is_empty() {
            return Ok(());
        }
        let map = SourceMap::new(FilePathMapping::empty());
        let file = map.new_source_file(FileName::Custom(origin.to_string()), src.to_string());
        let (tokens, symbols, lex_errors) = lex::lex(src, file.start_pos);
        if let Some(err) = lex_errors.first() {
            return Err(ConfigError::new(format!(
                "lexical error in `{origin}`: {err:?}"
            )));
        }
        let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
        let root = parser.try_file_scope().map_err(|err| {
            ConfigError::new(format!("syntax error in `{origin}`: {}", err.message()))
        })?;
        let ast = parser.finalize();
        let children = ast.get_children(root);
        let items = if children.is_empty() {
            &[][..]
        } else {
            ast.get_multi_child_slice(children[0]).unwrap_or(&[])
        };

        for &item in items {
            let Some(kind) = ast.get_node_kind(item) else {
                continue;
            };
            if kind != NodeKind::Configuration {
                return Err(ConfigError::new(format!(
                    "invalid item `{kind}` in `{origin}`: expected `.key value` configuration entry"
                )));
            }
            let slots = ast.get_children(item);
            let key_node = slots[0];
            let val_node = slots[1];
            let key = extract_id_text(&ast, key_node, src, file.start_pos.0).ok_or_else(|| {
                ConfigError::new(format!("invalid configuration key in `{origin}`"))
            })?;
            self.apply_ast_entry(&key, &ast, val_node, src, file.start_pos.0, origin)?;
        }
        Ok(())
    }

    fn apply_ast_entry(
        &mut self,
        key: &str,
        ast: &Ast,
        val_node: NodeIndex,
        src: &str,
        base_pos: u32,
        origin: &str,
    ) -> Result<(), ConfigError> {
        let val_kind = ast.get_node_kind(val_node).unwrap_or(NodeKind::Invalid);
        let raw_val = node_slice(ast, val_node, src, base_pos).trim();
        match key {
            "max_width" => {
                let n = parse_positive_usize(raw_val).ok_or_else(|| {
                    ConfigError::new(format!(
                        "`{origin}`: `.max_width` expects a positive integer, found `{raw_val}`"
                    ))
                })?;
                self.max_width = n;
            }
            "tab_spaces" => {
                let n = parse_positive_usize(raw_val).ok_or_else(|| {
                    ConfigError::new(format!(
                        "`{origin}`: `.tab_spaces` expects a positive integer, found `{raw_val}`"
                    ))
                })?;
                self.tab_spaces = n;
            }
            "newline_style" => {
                let style = if val_kind == NodeKind::Symbol {
                    let sym_id = ast.get_children(val_node)[0];
                    let name = extract_id_text(ast, sym_id, src, base_pos).unwrap_or_default();
                    NewlineStyle::parse_value(&name)
                } else {
                    NewlineStyle::parse_value(raw_val)
                }
                .ok_or_else(|| {
                    ConfigError::new(format!(
                        "`{origin}`: `.newline_style` expects `.lf`, `.crlf`, or `.auto`, found `{raw_val}`"
                    ))
                })?;
                self.newline_style = style;
            }
            "use_small_heuristics" => {
                let mode = if val_kind == NodeKind::Symbol {
                    let sym_id = ast.get_children(val_node)[0];
                    let name = extract_id_text(ast, sym_id, src, base_pos).unwrap_or_default();
                    SmallHeuristics::parse_value(&name)
                } else {
                    SmallHeuristics::parse_value(raw_val)
                }
                .ok_or_else(|| {
                    ConfigError::new(format!(
                        "`{origin}`: `.use_small_heuristics` expects `.default`, `.off`, or `.max`, found `{raw_val}`"
                    ))
                })?;
                self.use_small_heuristics = mode;
            }
            "reorder_imports" => {
                let b = parse_bool_literal(raw_val).ok_or_else(|| {
                    ConfigError::new(format!(
                        "`{origin}`: `.reorder_imports` expects `true` or `false`, found `{raw_val}`"
                    ))
                })?;
                self.reorder_imports = b;
            }
            "normalize_comments" => {
                let b = parse_bool_literal(raw_val).ok_or_else(|| {
                    ConfigError::new(format!(
                        "`{origin}`: `.normalize_comments` expects `true` or `false`, found `{raw_val}`"
                    ))
                })?;
                self.normalize_comments = b;
            }
            "ignore" => {
                if val_kind != NodeKind::ListOf {
                    return Err(ConfigError::new(format!(
                        "`{origin}`: `.ignore` expects a list of strings `[...]`, found `{raw_val}`"
                    )));
                }
                let slots = ast.get_children(val_node);
                let elems = if slots.is_empty() {
                    &[][..]
                } else {
                    ast.get_multi_child_slice(slots[0]).unwrap_or(&[])
                };
                let mut patterns = Vec::with_capacity(elems.len());
                for &elem in elems {
                    if ast.get_node_kind(elem) != Some(NodeKind::Str) {
                        let text = node_slice(ast, elem, src, base_pos);
                        return Err(ConfigError::new(format!(
                            "`{origin}`: `.ignore` elements must be string literals, found `{text}`"
                        )));
                    }
                    let lit = node_slice(ast, elem, src, base_pos).trim();
                    let decoded = unescape_flurry_string(lit).ok_or_else(|| {
                        ConfigError::new(format!(
                            "`{origin}`: invalid string literal `{lit}` in `.ignore`"
                        ))
                    })?;
                    patterns.push(decoded);
                }
                self.ignore = patterns;
            }
            other => {
                return Err(ConfigError::new(format!(
                    "`{origin}`: unknown configuration option `.{other}`"
                )));
            }
        }
        Ok(())
    }

    /// Apply CLI `--config` overrides. Accepts both `key=val,k2=v2` and `.key val, .k2 val2`.
    pub fn apply_cli_overrides(&mut self, spec: &str) -> Result<(), ConfigError> {
        let entries = split_top_level_commas(spec)?;
        for raw_entry in entries {
            let entry = raw_entry.trim();
            if entry.is_empty() {
                continue;
            }
            if let Some(rest) = entry.strip_prefix('.') {
                self.apply_dot_override(rest, "--config")?;
            } else if let Some((k, v)) = split_key_eq_val(entry) {
                self.apply_key_value_str(k.trim(), v.trim(), "--config")?;
            } else {
                return Err(ConfigError::new(format!(
                    "invalid `--config` entry `{entry}`: expected `key=value` or `.key value`"
                )));
            }
        }
        Ok(())
    }

    /// Apply a single `.key value` (or boolean `.key`) override from `--config` or `--%`.
    pub fn apply_dot_override(&mut self, rest: &str, origin: &str) -> Result<(), ConfigError> {
        let rest = rest.trim();
        let split_idx = rest
            .find(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or(rest.len());
        let key = &rest[..split_idx];
        let mut val = rest[split_idx..].trim();
        if let Some(after_eq) = val.strip_prefix('=') {
            val = after_eq.trim();
        }
        if key.is_empty() {
            return Err(ConfigError::new(format!(
                "`{origin}`: empty configuration key"
            )));
        }
        if val.is_empty() {
            // Boolean shorthand: `.reorder_imports` -> `true`
            match key {
                "reorder_imports" => {
                    self.reorder_imports = true;
                    return Ok(());
                }
                "normalize_comments" => {
                    self.normalize_comments = true;
                    return Ok(());
                }
                _ => {
                    return Err(ConfigError::new(format!(
                        "`{origin}`: option `.{key}` requires a value"
                    )));
                }
            }
        }
        self.apply_key_value_str(key, val, origin)
    }

    pub fn apply_key_value_str(
        &mut self,
        key: &str,
        val: &str,
        origin: &str,
    ) -> Result<(), ConfigError> {
        let key = key.strip_prefix('.').unwrap_or(key);
        match key {
            "max_width" => {
                let n = parse_positive_usize(val).ok_or_else(|| {
                    ConfigError::new(format!(
                        "`{origin}`: `max_width` expects a positive integer, found `{val}`"
                    ))
                })?;
                self.max_width = n;
            }
            "tab_spaces" => {
                let n = parse_positive_usize(val).ok_or_else(|| {
                    ConfigError::new(format!(
                        "`{origin}`: `tab_spaces` expects a positive integer, found `{val}`"
                    ))
                })?;
                self.tab_spaces = n;
            }
            "newline_style" => {
                let style = NewlineStyle::parse_value(val).ok_or_else(|| {
                    ConfigError::new(format!(
                        "`{origin}`: `newline_style` expects `.lf`, `.crlf`, or `.auto`, found `{val}`"
                    ))
                })?;
                self.newline_style = style;
            }
            "use_small_heuristics" => {
                let mode = SmallHeuristics::parse_value(val).ok_or_else(|| {
                    ConfigError::new(format!(
                        "`{origin}`: `use_small_heuristics` expects `.default`, `.off`, or `.max`, found `{val}`"
                    ))
                })?;
                self.use_small_heuristics = mode;
            }
            "reorder_imports" => {
                let b = parse_bool_literal(val).ok_or_else(|| {
                    ConfigError::new(format!(
                        "`{origin}`: `reorder_imports` expects `true` or `false`, found `{val}`"
                    ))
                })?;
                self.reorder_imports = b;
            }
            "normalize_comments" => {
                let b = parse_bool_literal(val).ok_or_else(|| {
                    ConfigError::new(format!(
                        "`{origin}`: `normalize_comments` expects `true` or `false`, found `{val}`"
                    ))
                })?;
                self.normalize_comments = b;
            }
            "ignore" => {
                let list = parse_string_list(val).ok_or_else(|| {
                    ConfigError::new(format!(
                        "`{origin}`: `ignore` expects `[\"pattern\", ...]`, found `{val}`"
                    ))
                })?;
                self.ignore = list;
            }
            other => {
                return Err(ConfigError::new(format!(
                    "`{origin}`: unknown configuration option `{other}`"
                )));
            }
        }
        Ok(())
    }

    /// Format configuration as valid `flurryfmt.fl` text.
    pub fn to_flurry_config_string(&self, minimal: bool) -> String {
        let def = Self::default();
        let mut lines = Vec::new();
        if !minimal || self.max_width != def.max_width {
            lines.push(format!(".max_width {},", self.max_width));
        }
        if !minimal || self.tab_spaces != def.tab_spaces {
            lines.push(format!(".tab_spaces {},", self.tab_spaces));
        }
        if !minimal || self.newline_style != def.newline_style {
            lines.push(format!(
                ".newline_style {},",
                self.newline_style.as_symbol_str()
            ));
        }
        if !minimal || self.use_small_heuristics != def.use_small_heuristics {
            lines.push(format!(
                ".use_small_heuristics {},",
                self.use_small_heuristics.as_symbol_str()
            ));
        }
        if !minimal || self.reorder_imports != def.reorder_imports {
            lines.push(format!(".reorder_imports {},", self.reorder_imports));
        }
        if !minimal || self.normalize_comments != def.normalize_comments {
            lines.push(format!(".normalize_comments {},", self.normalize_comments));
        }
        if !minimal || self.ignore != def.ignore {
            if self.ignore.is_empty() {
                lines.push(".ignore [],".to_string());
            } else {
                let quoted: Vec<String> = self
                    .ignore
                    .iter()
                    .map(|s| format!("\"{}\"", escape_flurry_string(s)))
                    .collect();
                lines.push(format!(".ignore [{}],", quoted.join(", ")));
            }
        }
        if lines.is_empty() {
            String::new()
        } else {
            let mut out = lines.join("\n");
            out.push('\n');
            out
        }
    }

    /// Check whether `path` is excluded by `.ignore` patterns.
    pub fn is_ignored(&self, path: &Path, scan_root: &Path) -> bool {
        if self.ignore.is_empty() {
            return false;
        }
        let mut candidates = Vec::new();
        if let Some(cfg_dir) = &self.config_dir {
            if let Ok(rel) = path.strip_prefix(cfg_dir) {
                candidates.push(normalize_slash_path(rel));
            }
        }
        if let Ok(rel) = path.strip_prefix(scan_root) {
            let norm = normalize_slash_path(rel);
            if !candidates.contains(&norm) {
                candidates.push(norm);
            }
        }
        let raw_norm = normalize_slash_path(path);
        if !candidates.contains(&raw_norm) {
            candidates.push(raw_norm);
        }

        for pattern in &self.ignore {
            let pat = pattern.trim();
            if pat.is_empty() {
                continue;
            }
            let pat = pat.strip_prefix("./").unwrap_or(pat);
            for candidate in &candidates {
                if candidate.is_empty() {
                    continue;
                }
                if matches_ignore_pattern(pat, candidate) {
                    return true;
                }
            }
        }
        false
    }
}

fn normalize_slash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn matches_ignore_pattern(pattern: &str, candidate: &str) -> bool {
    if let Some(dir_pat) = pattern.strip_suffix('/') {
        return candidate == dir_pat
            || candidate.starts_with(&format!("{dir_pat}/"))
            || glob_match(dir_pat, candidate)
            || glob_match(&format!("{dir_pat}/**"), candidate);
    }
    if !pattern.contains('/') {
        // Basename pattern (e.g. `"vendor"`, `"*.gen.fl"`) matches any path component.
        for seg in candidate.split('/') {
            if seg == pattern || glob_match(pattern, seg) {
                return true;
            }
        }
    }
    if !pattern.contains('*') && !pattern.contains('?') {
        if candidate == pattern || candidate.starts_with(&format!("{pattern}/")) {
            return true;
        }
    }
    glob_match(pattern, candidate)
}

/// Glob matcher supporting `*`, `**`, and `?` over `/`-separated paths.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p_bytes = pattern.as_bytes();
    let t_bytes = text.as_bytes();
    glob_match_inner(p_bytes, t_bytes)
}

fn glob_match_inner(pat: &[u8], text: &[u8]) -> bool {
    if pat.is_empty() {
        return text.is_empty();
    }
    if pat.starts_with(b"**/") {
        // Zero or more leading path segments.
        if glob_match_inner(&pat[3..], text) {
            return true;
        }
        for i in 0..text.len() {
            if text[i] == b'/' && glob_match_inner(&pat[3..], &text[i + 1..]) {
                return true;
            }
        }
        return false;
    }
    if pat == b"**" {
        return true;
    }
    if pat.starts_with(b"**") {
        return glob_match_inner(&pat[2..], text)
            || (!text.is_empty() && glob_match_inner(pat, &text[1..]));
    }
    if pat[0] == b'*' {
        if glob_match_inner(&pat[1..], text) {
            return true;
        }
        if !text.is_empty() && text[0] != b'/' {
            return glob_match_inner(pat, &text[1..]);
        }
        return false;
    }
    if pat[0] == b'?' {
        if !text.is_empty() && text[0] != b'/' {
            return glob_match_inner(&pat[1..], &text[1..]);
        }
        return false;
    }
    if !text.is_empty() && pat[0] == text[0] {
        return glob_match_inner(&pat[1..], &text[1..]);
    }
    false
}

fn extract_id_text(ast: &Ast, node: NodeIndex, src: &str, base_pos: u32) -> Option<String> {
    if ast.get_node_kind(node) != Some(NodeKind::Id) {
        return None;
    }
    let slots = ast.get_children(node);
    if slots.len() == 2 {
        // SAFETY: `Id` children were encoded by `Symbol::to_raw_parts` in this process.
        let sym = unsafe { symbol::Symbol::from_raw_parts(slots[0], slots[1]) };
        return Some(sym.as_str().to_string());
    }
    Some(node_slice(ast, node, src, base_pos).trim().to_string())
}

fn node_slice<'a>(ast: &Ast, node: NodeIndex, src: &'a str, base_pos: u32) -> &'a str {
    let Some(span) = ast.get_span(node) else {
        return "";
    };
    let lo = span.lo().0.saturating_sub(base_pos) as usize;
    let hi = span.hi().0.saturating_sub(base_pos) as usize;
    src.get(lo..hi).unwrap_or("")
}

fn parse_positive_usize(raw: &str) -> Option<usize> {
    let cleaned: String = raw.chars().filter(|&c| c != '_').collect();
    let val: usize = cleaned.parse().ok()?;
    (val >= 1).then_some(val)
}

fn parse_bool_literal(raw: &str) -> Option<bool> {
    match raw.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn parse_string_list(raw: &str) -> Option<Vec<String>> {
    let trimmed = raw.trim();
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?.trim();
    if inner.is_empty() {
        return Some(Vec::new());
    }
    let items = split_top_level_commas(inner).ok()?;
    let mut out = Vec::new();
    for item in items {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        if item.starts_with('"') && item.ends_with('"') {
            out.push(unescape_flurry_string(item)?);
        } else {
            // Also accept unquoted CLI shorthand like `ignore=[vendor,gen/**]`
            out.push(item.to_string());
        }
    }
    Some(out)
}

pub fn split_top_level_commas(input: &str) -> Result<Vec<&str>, ConfigError> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut bracket_depth = 0usize;

    for (idx, ch) in input.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '[' | '(' | '{' => bracket_depth += 1,
            ']' | ')' | '}' => {
                if bracket_depth == 0 {
                    return Err(ConfigError::new(format!(
                        "unmatched closing delimiter `{ch}` in `{input}`"
                    )));
                }
                bracket_depth -= 1;
            }
            ',' if bracket_depth == 0 => {
                parts.push(&input[start..idx]);
                start = idx + 1;
            }
            _ => {}
        }
    }
    if in_string {
        return Err(ConfigError::new(format!(
            "unterminated string literal in `{input}`"
        )));
    }
    if bracket_depth != 0 {
        return Err(ConfigError::new(format!(
            "unclosed bracket delimiter in `{input}`"
        )));
    }
    parts.push(&input[start..]);
    Ok(parts)
}

fn split_key_eq_val(entry: &str) -> Option<(&str, &str)> {
    let mut in_string = false;
    let mut escaped = false;
    for (idx, ch) in entry.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
        } else if ch == '=' {
            return Some((&entry[..idx], &entry[idx + 1..]));
        }
    }
    None
}

fn unescape_flurry_string(lit: &str) -> Option<String> {
    let inner = lit.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next()? {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '\\' => out.push('\\'),
            '\'' => out.push('\''),
            '"' => out.push('"'),
            '0' => out.push('\0'),
            '$' => out.push('$'),
            'x' => {
                if chars.next()? != '{' {
                    return None;
                }
                let mut hex = String::new();
                while let Some(&c) = chars.peek() {
                    if c == '}' {
                        chars.next();
                        break;
                    }
                    hex.push(c);
                    chars.next();
                }
                let code = u32::from_str_radix(&hex, 16).ok()?;
                out.push(char::from_u32(code)?);
            }
            _ => return None,
        }
    }
    Some(out)
}

fn escape_flurry_string(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            '$' => out.push_str("\\$"),
            c => out.push(c),
        }
    }
    out
}
