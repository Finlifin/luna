//! List descriptors retain ordinary declared domains; no library spelling is used.
use super::*;
#[derive(Debug)]
pub struct Construction {
    pub ty: Term,
    pub inputs: Vec<Term>,
}
impl fmt::Display for Construction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(list-construction (type {}) (inputs", self.ty)?;
        for input in &self.inputs {
            write!(f, " {input}")?;
        }
        write!(f, "))")
    }
}
#[derive(Debug)]
pub struct Operation {
    pub ty: Term,
    pub left: Term,
    pub right: Option<Term>,
}
impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(list-operation (type {}) (left {}) (right",
            self.ty, self.left
        )?;
        if let Some(right) = &self.right {
            write!(f, " {right}")?;
        }
        write!(f, "))")
    }
}

#[derive(Debug)]
pub struct Index {
    pub ty: Term,
    pub list: Term,
    pub index: Term,
}
impl fmt::Display for Index {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(list-index-contract (type {}) (list {}) (index {}))",
            self.ty, self.list, self.index
        )
    }
}

/// These operands cannot run a callback or mutate the destination between reads.
pub(crate) fn inert_suffix(expr: &Expr, fuel: &mut usize) -> Result<bool, Error> {
    Ok(match &expr.kind {
        K::TypeList(fields) => {
            *fuel = fuel
                .checked_sub(fields.len())
                .ok_or(Error::Budget(expr.source))?;
            fields.iter().all(|field| {
                matches!(
                    field.kind,
                    K::Local(_) | K::TypeValue(_) | K::ComptimeValue(_) | K::Literal(_)
                )
            })
        }
        K::TypeListValue(_) | K::ComptimeValue(_) | K::Local(_) => true,
        _ => false,
    })
}
