//! Macro registry, visibility descriptors, and execution host trait.
use crate::ast_input::{AstArgValue, AstParamShape};
use crate::provenance::{MacroToken, escape_sexp_str};
use crate::{MacroBudget, MacroError};
use rustc_span::Span;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Stable definition identity for a macro (`package`, `local` definition index).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MacroId {
    pub package: u32,
    pub local: u32,
}

impl MacroId {
    /// Synthetic identity reserved for built-in `@builtin.str.raw`.
    pub const BUILTIN_RAW_STR: Self = Self {
        package: u32::MAX,
        local: 1,
    };

    /// Synthetic identity reserved for built-in `@meta.macros.define_macro1`.
    pub const BUILTIN_DEFINE_MACRO1: Self = Self {
        package: u32::MAX,
        local: 2,
    };
}

impl fmt::Display for MacroId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(macro-id {} {})", self.package, self.local)
    }
}

/// Visibility level enforced when importing or invoking a construction macro.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MacroVisibility {
    Public,
    Package(u32),
    Private,
}

impl MacroVisibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Package(_) => "package",
            Self::Private => "private",
        }
    }
}

impl fmt::Display for MacroVisibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Public => f.write_str("public"),
            Self::Package(pkg) => write!(f, "(package {pkg})"),
            Self::Private => f.write_str("private"),
        }
    }
}

/// Flurry AST node categories selectable by `@define_macro1` signatures and PEG rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AstNodeKind {
    FlurryPattern,
    FlurryExpr,
    FlurryStatement,
    FlurryExtendedStatement,
    FlurryItem,
    FlurryType,
    FlurryIdent,
    FlurryLiteral,
    FlurryBlock,
    FlurryPath,
}

impl AstNodeKind {
    pub fn from_type_name(name: &str) -> Option<Self> {
        Some(match name {
            "FlurryPattern" => Self::FlurryPattern,
            "FlurryExpr" => Self::FlurryExpr,
            "FlurryStatement" => Self::FlurryStatement,
            "FlurryExtendedStatement" => Self::FlurryExtendedStatement,
            "FlurryItem" => Self::FlurryItem,
            "FlurryType" => Self::FlurryType,
            "FlurryIdent" => Self::FlurryIdent,
            "FlurryLiteral" => Self::FlurryLiteral,
            "FlurryBlock" => Self::FlurryBlock,
            "FlurryPath" => Self::FlurryPath,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::FlurryPattern => "FlurryPattern",
            Self::FlurryExpr => "FlurryExpr",
            Self::FlurryStatement => "FlurryStatement",
            Self::FlurryExtendedStatement => "FlurryExtendedStatement",
            Self::FlurryItem => "FlurryItem",
            Self::FlurryType => "FlurryType",
            Self::FlurryIdent => "FlurryIdent",
            Self::FlurryLiteral => "FlurryLiteral",
            Self::FlurryBlock => "FlurryBlock",
            Self::FlurryPath => "FlurryPath",
        }
    }
}

impl fmt::Display for AstNodeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Construction macro input protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MacroInputMode {
    /// Built-in `@builtin.str.raw { raw_source }` encoded into a `str` literal token.
    BuiltinRawStr,
    /// Built-in `@meta.macros.define_macro1 { mapping_definition }`.
    BuiltinDefineMacro1,
    /// Mode 1 AST construction macro defined via `@define_macro1`.
    Ast { params: Vec<AstParamShape> },
    /// User Mode 4 construction macro (`^macro4`) receiving raw UTF-8 `str` inside `{ ... }`.
    RawSource,
    /// User Mode 3 construction macro (`^macro3`) receiving `meta.ast.TokenBuffer` inside `{ ... }`.
    Tokens,
}

impl MacroInputMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BuiltinRawStr => "builtin-raw-str",
            Self::BuiltinDefineMacro1 => "builtin-define-macro1",
            Self::Ast { .. } => "ast",
            Self::RawSource => "raw-source",
            Self::Tokens => "tokens",
        }
    }
}

impl fmt::Display for MacroInputMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ast { params } => {
                f.write_str("(ast")?;
                for param in params {
                    write!(f, " {param}")?;
                }
                f.write_str(")")
            }
            other => f.write_str(other.as_str()),
        }
    }
}

/// Registered construction macro metadata available to the expander.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MacroDescriptor {
    pub id: MacroId,
    pub canonical_path: Vec<String>,
    pub package: u32,
    pub module_path: Vec<String>,
    pub vis: MacroVisibility,
    pub def_site: Option<Span>,
    pub mode: MacroInputMode,
}

impl MacroDescriptor {
    pub fn full_path(&self) -> String {
        self.canonical_path.join(".")
    }

    pub fn is_accessible_from(&self, current_package: u32, current_module: &[String]) -> bool {
        match self.vis {
            MacroVisibility::Public => true,
            MacroVisibility::Package(pkg) => pkg == current_package,
            MacroVisibility::Private => {
                self.package == current_package && self.module_path == current_module
            }
        }
    }
}

impl fmt::Display for MacroDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(macro-descriptor (id {}) (path \"{}\") (vis {}) (mode {}))",
            self.id,
            escape_sexp_str(&self.full_path()),
            self.vis,
            self.mode
        )
    }
}

/// Registry of built-in, script-defined, and dependency-exported construction macros.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MacroRegistry {
    by_id: BTreeMap<MacroId, MacroDescriptor>,
    by_path: BTreeMap<Vec<String>, MacroId>,
    packages: BTreeSet<String>,
}

impl MacroRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registry populated with built-in macros such as `builtin.str.raw` and `meta.macros.define_macro1`.
    pub fn with_builtins() -> Self {
        let mut reg = Self::new();
        reg.register_module_root("builtin");
        reg.register_module_root("meta");
        reg.register(MacroDescriptor {
            id: MacroId::BUILTIN_RAW_STR,
            canonical_path: vec!["builtin".into(), "str".into(), "raw".into()],
            package: u32::MAX,
            module_path: vec!["builtin".into(), "str".into()],
            vis: MacroVisibility::Public,
            def_site: None,
            mode: MacroInputMode::BuiltinRawStr,
        });
        reg.register(MacroDescriptor {
            id: MacroId::BUILTIN_DEFINE_MACRO1,
            canonical_path: vec!["meta".into(), "macros".into(), "define_macro1".into()],
            package: u32::MAX,
            module_path: vec!["meta".into(), "macros".into()],
            vis: MacroVisibility::Public,
            def_site: None,
            mode: MacroInputMode::BuiltinDefineMacro1,
        });
        reg
    }

    pub fn register_module_root(&mut self, name: impl Into<String>) {
        self.packages.insert(name.into());
    }

    pub fn register(&mut self, descriptor: MacroDescriptor) {
        if let Some(root) = descriptor.canonical_path.first() {
            self.packages.insert(root.clone());
        }
        let id = descriptor.id;
        self.by_path.insert(descriptor.canonical_path.clone(), id);
        self.by_id.insert(id, descriptor);
    }

    /// Alias a path (for example via `pub use` re-exports) to an existing macro identity.
    pub fn register_path_alias(&mut self, path: Vec<String>, id: MacroId) {
        if self.by_id.contains_key(&id) {
            self.by_path.insert(path, id);
        }
    }

    pub fn get(&self, id: MacroId) -> Option<&MacroDescriptor> {
        self.by_id.get(&id)
    }

    pub fn lookup_path(&self, path: &[String]) -> Option<&MacroDescriptor> {
        let id = self.by_path.get(path)?;
        self.by_id.get(id)
    }

    pub fn lookup_id(&self, path: &[String]) -> Option<MacroId> {
        self.by_path.get(path).copied()
    }

    pub fn module_exists(&self, prefix: &[String]) -> bool {
        if prefix.is_empty() {
            return false;
        }
        if prefix.len() == 1 && self.packages.contains(&prefix[0]) {
            return true;
        }
        self.by_path
            .keys()
            .any(|path| path.len() > prefix.len() && path[..prefix.len()] == *prefix)
    }

    /// Direct member names and macro descriptors under `prefix.*`.
    pub fn glob_members(&self, prefix: &[String]) -> Vec<(String, Option<MacroId>, Vec<String>)> {
        let mut results = Vec::new();
        let mut seen_submodules = BTreeSet::new();
        for (path, &id) in &self.by_path {
            if path.len() == prefix.len() + 1 && path[..prefix.len()] == *prefix {
                results.push((path[prefix.len()].clone(), Some(id), path.clone()));
            } else if path.len() > prefix.len() + 1 && path[..prefix.len()] == *prefix {
                let child_mod = path[prefix.len()].clone();
                if seen_submodules.insert(child_mod.clone()) {
                    let mut mod_path = prefix.to_vec();
                    mod_path.push(child_mod.clone());
                    results.push((child_mod, None, mod_path));
                }
            }
        }
        results
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

impl fmt::Display for MacroRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(macro-registry")?;
        for desc in self.by_id.values() {
            write!(f, " {desc}")?;
        }
        f.write_str(")")
    }
}

/// Compiler host bridge for evaluating `^macro1`–`^macro4` construction handlers at comptime.
pub trait MacroExecutor {
    fn execute_raw_source(
        &mut self,
        descriptor: &MacroDescriptor,
        raw_source: &str,
        call_site: Span,
        budget: &mut MacroBudget,
    ) -> Result<Vec<MacroToken>, MacroError>;

    fn execute_tokens(
        &mut self,
        descriptor: &MacroDescriptor,
        input_tokens: &[MacroToken],
        call_site: Span,
        budget: &mut MacroBudget,
    ) -> Result<Vec<MacroToken>, MacroError>;

    fn execute_ast(
        &mut self,
        descriptor: &MacroDescriptor,
        args: &[AstArgValue],
        call_site: Span,
        budget: &mut MacroBudget,
    ) -> Result<Vec<MacroToken>, MacroError>;
}

/// Host context supplied to `expand_source_with_context`.
pub struct ExpansionContext<'a> {
    pub current_package: u32,
    pub current_module_path: Vec<String>,
    pub registry: &'a MacroRegistry,
    pub executor: Option<&'a mut dyn MacroExecutor>,
}

impl<'a> ExpansionContext<'a> {
    pub fn with_registry(registry: &'a MacroRegistry) -> Self {
        Self {
            current_package: 0,
            current_module_path: Vec::new(),
            registry,
            executor: None,
        }
    }
}
