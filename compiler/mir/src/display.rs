//! MIR pretty-printing.

use std::fmt;

use crate::*;

impl<'tcx> Body<'tcx> {
    /// Pretty-print the entire MIR body.
    pub fn dump(&self) -> String {
        let mut out = String::new();

        // Header
        let name = self
            .local_decls
            .first()
            .and_then(|d| d.name.as_deref())
            .unwrap_or("?");
        out.push_str(&format!("fn {}(", name));

        // Parameters
        for i in 1..=self.arg_count {
            if i > 1 {
                out.push_str(", ");
            }
            let decl = &self.local_decls[i];
            let param_name = decl.name.as_deref().unwrap_or("_");
            out.push_str(&format!("{}: {}", param_name, decl.ty));
        }

        let ret_ty = &self.local_decls[0].ty;
        out.push_str(&format!(") -> {} {{\n", ret_ty));

        // Local declarations (temporaries)
        for (i, decl) in self.local_decls.iter().enumerate() {
            let local = Local::new(i as u32);
            let comment = decl.name.as_deref().unwrap_or("");
            if i == 0 {
                out.push_str(&format!("    let {}: {};  // return place\n", local, decl.ty));
            } else if i <= self.arg_count {
                continue; // already shown in signature
            } else {
                if comment.is_empty() {
                    out.push_str(&format!("    let {}: {};\n", local, decl.ty));
                } else {
                    out.push_str(&format!(
                        "    let {}: {};  // {}\n",
                        local, decl.ty, comment
                    ));
                }
            }
        }
        out.push('\n');

        // Basic blocks
        for (i, bb) in self.basic_blocks.iter().enumerate() {
            out.push_str(&format!("    bb{}: {{\n", i));
            for stmt in &bb.statements {
                out.push_str(&format!("        {};\n", stmt));
            }
            if let Some(ref term) = bb.terminator {
                out.push_str(&format!("        {};\n", term));
            }
            out.push_str("    }\n\n");
        }

        out.push_str("}\n");
        out
    }
}

// ── Display impls ────────────────────────────────────────────────────────────

impl fmt::Display for Statement<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            StatementKind::Assign(place, rvalue) => write!(f, "{} = {}", place, rvalue),
            StatementKind::Nop => write!(f, "nop"),
        }
    }
}

impl fmt::Display for Terminator<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            TerminatorKind::Goto { target } => write!(f, "goto -> {}", target),
            TerminatorKind::Return => write!(f, "return"),
            TerminatorKind::Unreachable => write!(f, "unreachable"),
            TerminatorKind::SwitchInt { discr, targets } => {
                write!(f, "switchInt({}) -> [", discr)?;
                for (i, (val, bb)) in targets.values.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", val, bb)?;
                }
                write!(f, ", otherwise: {}]", targets.otherwise)
            }
            TerminatorKind::Call {
                func,
                args,
                destination,
                target,
            } => {
                write!(f, "{} = {}(", destination, func)?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", arg)?;
                }
                write!(f, ")")?;
                if let Some(t) = target {
                    write!(f, " -> {}", t)?;
                }
                Ok(())
            }
        }
    }
}

impl fmt::Display for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.local)?;
        for proj in &self.projection {
            match proj {
                PlaceElem::Field(idx) => write!(f, ".{}", idx)?,
                PlaceElem::Deref => write!(f, ".*")?,
                PlaceElem::Index(local) => write!(f, "[{}]", local)?,
            }
        }
        Ok(())
    }
}

impl fmt::Display for Operand<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operand::Copy(place) => write!(f, "copy {}", place),
            Operand::Move(place) => write!(f, "move {}", place),
            Operand::Constant(c) => write!(f, "{}", c),
        }
    }
}

impl fmt::Display for Rvalue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rvalue::Use(op) => write!(f, "{}", op),
            Rvalue::BinaryOp(op, lhs, rhs) => write!(f, "{}({}, {})", op, lhs, rhs),
            Rvalue::UnaryOp(op, operand) => write!(f, "{}({})", op, operand),
            Rvalue::Ref(place) => write!(f, "&{}", place),
            Rvalue::Aggregate(kind, ops) => {
                match kind {
                    AggregateKind::Tuple => write!(f, "(")?,
                    AggregateKind::Array => write!(f, "[")?,
                    AggregateKind::Adt(def) => write!(f, "Adt({:?})(", def)?,
                }
                for (i, op) in ops.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", op)?;
                }
                match kind {
                    AggregateKind::Tuple | AggregateKind::Adt(_) => write!(f, ")"),
                    AggregateKind::Array => write!(f, "]"),
                }
            }
        }
    }
}

impl fmt::Display for Constant<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ConstKind::Int(v) => write!(f, "const {}_{}", v, self.ty),
            ConstKind::Float(v) => write!(f, "const {}_{}", v, self.ty),
            ConstKind::Bool(v) => write!(f, "const {}", v),
            ConstKind::Char(v) => write!(f, "const '{}'", v),
            ConstKind::Str(v) => write!(f, "const \"{}\"", v),
            ConstKind::FnDef(def) => write!(f, "fn_def({:?})", def),
            ConstKind::FnName(name) => write!(f, "fn({})", name),
            ConstKind::Unit => write!(f, "const ()"),
            ConstKind::Null => write!(f, "const null"),
        }
    }
}
