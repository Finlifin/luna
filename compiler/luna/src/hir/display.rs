use super::*;
use std::fmt::{Display, Formatter, Result};

impl<'hir> Display for Expr<'hir> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            // Literals
            Expr::IntLiteral(n) => write!(f, "(IntLiteral {})", n),
            Expr::BoolLiteral(b) => write!(f, "(BoolLiteral {})", b),
            Expr::RealLiteral(i, u) => write!(f, "(RealLiteral {}.{})", i, u),
            Expr::StrLiteral(s) => write!(f, "(StrLiteral \"{}\")", s),
            Expr::CharLiteral(c) => write!(f, "(CharLiteral '{}')", c),
            Expr::SymbolLiteral(s) => write!(f, "(SymbolLiteral :{})", s),

            // Special values
            Expr::Null => write!(f, "(Null)"),
            Expr::Undefined => write!(f, "(Undefined)"),
            Expr::Unit => write!(f, "(unit)"),
            Expr::Any => write!(f, "(any)"),
            Expr::SelfVal => write!(f, "(self)"),
            Self::TySelf => write!(f, "(SelfType)"),

            // References
            Expr::Ref(id) => write!(f, "(Ref {})", id),

            // Collections
            Expr::List(exprs) => {
                let items = exprs
                    .iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                write!(f, "(List {})", items)
            }
            Expr::Tuple(exprs) => {
                let items = exprs
                    .iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                write!(f, "(Tuple {})", items)
            }
            Expr::Object(exprs, props) => {
                let expr_items = exprs
                    .iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                let prop_items = props
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                write!(f, "(Object {} {})", expr_items, prop_items)
            }
            Expr::Range {
                from,
                to,
                inclusive,
            } => {
                write!(f, "(Range {} {} {})", from, to, inclusive)
            }
            Expr::Pattern(p) => write!(f, "(Pattern {})", p),

            // Function calls
            Expr::FnApply {
                callee,
                args,
                optional_args,
            } => {
                let arg_items = args
                    .iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                let opt_items = optional_args
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                write!(f, "(FnApply {} {} {})", callee, arg_items, opt_items)
            }
            Expr::NormalFormFnApply {
                callee,
                args,
                optional_args,
            } => {
                let arg_items = args
                    .iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                let opt_items = optional_args
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                write!(
                    f,
                    "(NormalFormFnApply {} {} {})",
                    callee, arg_items, opt_items
                )
            }
            Expr::FnObjectApply {
                callee,
                elements,
                properties,
            } => {
                let elem_items = elements
                    .iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                let prop_items = properties
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                write!(
                    f,
                    "(FnObjectApply {} {} {})",
                    callee, elem_items, prop_items
                )
            }
            Expr::UnaryApply { expr, op } => {
                write!(f, "(UnaryApply {} {})", op, expr)
            }
            Expr::BinaryApply { left, right, op } => {
                write!(f, "(BinaryApply {} {} {})", op, left, right)
            }
            Expr::ObjectApply {
                callee,
                args,
                optional_args,
                object,
            } => {
                let arg_items = args
                    .iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                let opt_items = optional_args
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                write!(
                    f,
                    "(ObjectApply {} {} {} {})",
                    callee, arg_items, opt_items, object
                )
            }
            Expr::Index(expr, index) => write!(f, "(Index {} {})", expr, index),
            Expr::Matches(expr, pattern) => write!(f, "(Matches {} {})", expr, pattern),

            // Control flow
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => match else_branch {
                Some(else_expr) => write!(f, "(If {} {} {})", condition, then_branch, else_expr),
                None => write!(f, "(If {} {})", condition, then_branch),
            },
            Expr::When {
                conditions,
                branches,
            } => {
                let cond_items = conditions
                    .iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                let branch_items = branches
                    .iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                write!(f, "(When {} {})", cond_items, branch_items)
            }
            Expr::Match { subject, arms } => {
                let arm_items = arms
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                write!(f, "(Match {} {})", subject, arm_items)
            }
            Expr::While { condition, body } => {
                write!(f, "(While {} {})", condition, body)
            }
            Expr::For {
                pattern,
                iterable,
                body,
            } => {
                write!(f, "(For {} {} {})", pattern, iterable, body)
            }
            Expr::Let {
                pattern,
                ty: value,
                init: body,
            } => {
                write!(f, "(Let {} {} {})", pattern, value, body)
            }
            Expr::Const {
                pattern,
                ty: value,
                init: body,
            } => {
                write!(f, "(Const {} {} {})", pattern, value, body)
            }
            Expr::Assign { location, value } => {
                write!(f, "(Assign {} {})", location, value)
            }
            Expr::Block(kind, exprs) => {
                let items = exprs
                    .iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                write!(f, "(Block {:?} {})", kind, items)
            }
            Expr::ExprStatement(expr) => write!(f, "(ExprStatement {})", expr),
            Expr::Break(label) => match label {
                Some(l) => write!(f, "(Break {})", l),
                None => write!(f, "(Break)"),
            },
            Expr::Continue(label) => match label {
                Some(l) => write!(f, "(Continue {})", l),
                None => write!(f, "(Continue)"),
            },
            Expr::Return(expr) => match expr {
                Some(e) => write!(f, "(Return {})", e),
                None => write!(f, "(Return)"),
            },
            Expr::Resume(expr) => match expr {
                Some(e) => write!(f, "(Resume {})", e),
                None => write!(f, "(Resume)"),
            },

            // Types
            Expr::TyVoid => write!(f, "(TyVoid)"),
            Expr::TyNoReturn => write!(f, "(TyNoReturn)"),
            Expr::TyAny => write!(f, "(TyAny)"),
            Expr::TyInteger => write!(f, "(TyInteger)"),
            Expr::TyReal => write!(f, "(TyReal)"),
            Expr::TyChar => write!(f, "(TyChar)"),
            Expr::TySymbol => write!(f, "(TySymbol)"),
            Expr::TyObject => write!(f, "(TyObject)"),
            Expr::TyStr => write!(f, "(TyStr)"),
            Expr::TyBool => write!(f, "(TyBool)"),
            Expr::TyInt(bits, signed) => write!(f, "(TyInt {} {})", bits, signed),
            Expr::TyFloat(bits) => write!(f, "(TyFloat {})", bits),
            Expr::TyOptional(ty) => write!(f, "(TyOptional {})", ty),
            Expr::TyTuple(types) => {
                let items = types
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                write!(f, "(TyTuple {})", items)
            }
            Expr::TyPointer(ty) => write!(f, "(TyPointer {})", ty),
            Expr::TyArray(ty, size) => write!(f, "(TyArray {} {})", ty, size),
            Expr::TyScheme(params, ty) => {
                let param_items = params
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                write!(f, "(TyScheme {} {})", param_items, ty)
            }
            Expr::TyNamed(name, def) => write!(f, "(TyNamed {} {})", name, def),
            Expr::TyAlias(name, ty) => write!(f, "(TyAlias {} {})", name, ty),

            Expr::Select(left, id) => write!(f, "(Select {} {})", left, id),
        }
    }
}

impl<'hir> Display for Pattern<'hir> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            Pattern::Ref(id) => write!(f, "(Ref {})", id),
            Pattern::Wildcard => write!(f, "(Wildcard)"),
            Pattern::Literal(expr) => write!(f, "(Literal {})", expr),
            Pattern::Symbol(sym) => write!(f, "(Symbol {})", sym),

            Pattern::Null => write!(f, "(Null)"),
            Pattern::Some(pat) => write!(f, "(Some {})", pat),

            Pattern::Ok(pat) => write!(f, "(Ok {})", pat),
            Pattern::Err(pat) => write!(f, "(Err {})", pat),

            Pattern::Range {
                from,
                to,
                inclusive,
            } => write!(f, "(Range {} {} {})", from, to, inclusive),

            Pattern::TupleDestructure(m) => write!(
                f,
                "(TupleDestructure {})",
                m.iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            Pattern::CallDestructure(e, m) => write!(
                f,
                "(CallDestructure {} {})",
                e,
                m.iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            ),

            Pattern::ListDestructure(m) => write!(
                f,
                "(ListDestructure {})",
                m.iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            Pattern::ObjectDestructure(m) => write!(
                f,
                "(ObjectDestructure {})",
                m.iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            Pattern::ObjectCallDestructure(e, m) => write!(
                f,
                "(ObjectCallDestructure {} {})",
                e,
                m.iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            ),

            Pattern::Property(sym, pat) => write!(f, "(Property {} {})", sym, pat),

            Pattern::Variable(pat) => write!(f, "(Variable {})", pat),
        }
    }
}

impl<'hir> Display for Definition<'hir> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            Definition::Module(module) => write!(f, "(Module {})", module),
            Definition::Struct(struct_def) => write!(f, "(Struct {})", struct_def),
            Definition::StructField(name, ty, default) => match default {
                Some(def) => write!(f, "(StructField {} {} {})", name, ty, def),
                None => write!(f, "(StructField {} {})", name, ty),
            },
            Definition::Enum(enum_def) => write!(f, "(Enum {})", enum_def),
            Definition::EnumVariant(name) => write!(f, "(EnumVariant {})", name),
            Definition::EnumVariantWithStruct(name, fields) => {
                let field_items = fields
                    .iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                write!(f, "(EnumVariantWithStruct {} {})", name, field_items)
            }
            Definition::EnumVariantWithTuple(name, types) => {
                let type_items = types
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                write!(f, "(EnumVariantWithTuple {} {})", name, type_items)
            }
            Definition::EnumVariantWithPattern(name, pattern) => {
                write!(f, "(EnumVariantWithPattern {} {})", name, pattern)
            }
            Definition::EnumVariantWithSubEnum(name, variants) => {
                let variant_items = variants
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                write!(f, "(EnumVariantWithSubEnum {} {})", name, variant_items)
            }
            Definition::Function(func) => write!(f, "(Function {})", func),
            Definition::Intrinsic(intrinsic) => write!(f, "(Intrinsic {})", intrinsic.to_str()),
            Definition::FileScope {
                name,
                items,
                scope_id,
            } => {
                let item_list = items
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                write!(f, "(FileScope {} {} {})", name, scope_id, item_list)
            }
            Definition::Package { name, scope_id } => {
                write!(f, "(Package {} {})", name, scope_id)
            }
        }
    }
}

impl<'hir> Display for Function<'hir> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        let clause_items = self
            .clauses
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let param_items = self
            .params
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        write!(
            f,
            "(Function {} {} {} {} {})",
            self.kind, self.name, clause_items, param_items, self.body
        )
    }
}

impl Display for FnKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            FnKind::Normal => write!(f, "Normal"),
            FnKind::Method => write!(f, "Method"),
            FnKind::RefMethod => write!(f, "RefMethod"),
        }
    }
}

impl<'hir> Display for Clause<'hir> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            Clause::TypeDecl {
                symbol, self_id, ..
            } => {
                write!(f, "(TypeDecl {} {})", symbol, self_id)
            }
            Clause::TypeTraitBounded {
                symbol,
                trait_bound,
                self_id,
                ..
            } => {
                write!(
                    f,
                    "(TypeTraitBounded {} {} {})",
                    symbol, trait_bound, self_id
                )
            }
            Clause::Decl {
                symbol,
                ty,
                default,
                self_id,
                ..
            } => match default {
                Some(def) => write!(f, "(Decl {} {} {} {})", symbol, ty, def, self_id),
                None => write!(f, "(Decl {} {} {})", symbol, ty, self_id),
            },
            Clause::Requires => write!(f, "(Requires)"),
            Clause::Ensures => write!(f, "(Ensures)"),
            Clause::Decreases => write!(f, "(Decreases)"),
            Clause::Outcomes => write!(f, "(Outcomes)"),
        }
    }
}

impl<'hir> Display for Param<'hir> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            Param::Itself { is_ref } => write!(f, "(Itself {})", is_ref),
            Param::Typed(name, ty, default) => match default {
                Some(def) => write!(f, "(Typed {} {} {})", name, ty, def),
                None => write!(f, "(Typed {} {})", name, ty),
            },
            Param::AutoCollectToTuple(name, ty) => {
                write!(f, "(AutoCollectToTuple {} {})", name, ty)
            }
            Param::AutoCollectToObject(name, ty) => {
                write!(f, "(AutoCollectToObject {} {})", name, ty)
            }
        }
    }
}

impl<'hir> Display for Module<'hir> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        let clause_items = self
            .clauses
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        write!(
            f,
            "(Module {} {} {})",
            self.name, clause_items, self.scope_id
        )
    }
}

impl<'hir> Display for Struct<'hir> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        let clause_items = self
            .clauses
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let field_items = self
            .fields
            .iter()
            .map(|field| field.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        write!(
            f,
            "(Struct {} {} {} {})",
            self.name, clause_items, field_items, self.scope_id
        )
    }
}

impl<'hir> Display for Enum<'hir> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        let clause_items = self
            .clauses
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let variant_items = self
            .variants
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let item_items = self
            .items
            .iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        write!(
            f,
            "(Enum {} {} {} {})",
            self.name, clause_items, variant_items, item_items
        )
    }
}

impl Display for BinaryOp {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            BinaryOp::Add => write!(f, "Add"),
            BinaryOp::Sub => write!(f, "Sub"),
            BinaryOp::Div => write!(f, "Div"),
            BinaryOp::Mul => write!(f, "Mul"),
            BinaryOp::Mod => write!(f, "Mod"),
            BinaryOp::BoolAnd => write!(f, "BoolAnd"),
            BinaryOp::BoolOr => write!(f, "BoolOr"),
            BinaryOp::BoolEq => write!(f, "BoolEq"),
            BinaryOp::BoolNotEq => write!(f, "BoolNotEq"),
            BinaryOp::BoolGt => write!(f, "BoolGt"),
            BinaryOp::BoolLt => write!(f, "BoolLt"),
            BinaryOp::BoolGtEq => write!(f, "BoolGtEq"),
            BinaryOp::BoolLtEq => write!(f, "BoolLtEq"),
            BinaryOp::BoolImplies => write!(f, "BoolImplies"),
            BinaryOp::BoolTypedWith => write!(f, "BoolTypedWith"),
            BinaryOp::BoolTraitBound => write!(f, "BoolTraitBound"),
            BinaryOp::AddAdd => write!(f, "AddAdd"),
        }
    }
}

impl Display for UnaryOp {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            UnaryOp::Neg => write!(f, "Neg"),
            UnaryOp::Not => write!(f, "Not"),
            UnaryOp::Refer => write!(f, "Refer"),
            UnaryOp::Deref => write!(f, "Deref"),
        }
    }
}

impl<'hir> Display for Property<'hir> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "(Property {} {})", self.name, self.value)
    }
}

impl<'hir> Display for Import<'hir> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            Import::All(scope_id) => write!(f, "(ImportAll {})", scope_id),
            Import::Multi(scope_id, symbols) => {
                let symbol_items = symbols
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                write!(f, "(ImportMulti {} {})", scope_id, symbol_items)
            }
            Import::Single(scope_id, symbol) => {
                write!(f, "(ImportSingle {} {})", scope_id, symbol)
            }
            Import::Alias {
                scope_id,
                alias,
                original,
            } => {
                write!(f, "(ImportAlias {} {} {})", scope_id, alias, original)
            }
        }
    }
}

impl<'hir> Display for HirMapping<'hir> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            HirMapping::Expr(expr, id) => write!(f, "(HirExpr {} {})", expr, id),
            HirMapping::Pattern(pattern, id) => write!(f, "(HirPattern {} {})", pattern, id),
            HirMapping::Definition(def, id) => write!(f, "(HirDefinition {} {})", def, id),
            HirMapping::Param(param, id) => write!(f, "(HirParam {} {})", param, id),
            HirMapping::Clause(clause, id) => write!(f, "(HirClause {} {})", clause, id),
            HirMapping::Unresolved(node_id, node_index, id) => {
                write!(f, "(HirUnresolved {} {} {})", node_id, node_index, id)
            }
            HirMapping::UnresolvedFileScope(node_id, id) => {
                write!(f, "(HirUnresolvedFileScope {} {})", node_id, id)
            }
            HirMapping::UnresolvedPackage(node_id) => {
                write!(f, "(HirUnresolvedPackage {})", node_id)
            }
            HirMapping::UnresolvedDirectoryModule(node_id, id) => {
                write!(f, "(HirUnresolvedDirectoryModule {} {})", node_id, id)
            }
            HirMapping::Invalid => write!(f, "(Invalid)"),

            _ => write!(f, "(Todo {:?})", self),
        }
    }
}
