//! Installation-value delivery decisions published by resource elaboration.
//! The classifier is a trusted compiler policy, like cleanup publication; this
//! module checks graph binding and occurrence contracts, not source trait proofs.
use crate::type_links::TypeLinks;
use crate::{Package, visit};
use nir::{
    Term,
    effects::{Abi, Operation},
    program::ExprKind,
};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    Move,
    Copy,
}
impl fmt::Display for Delivery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Move => "move",
            Self::Copy => "copy",
        })
    }
}
struct Entry {
    operation: Operation,
    result: Term,
    delivery: Delivery,
}

/// Borrowing the graph prevents mutation while the acquired decisions are live.
/// A plan is intentionally not transferable to a different graph, even an equal
/// clone: that graph must acquire its own decisions in its compilation world.
pub struct Plan<'a> {
    graph: &'a Package,
    links: TypeLinks<'a>,
    entries: Vec<Entry>,
}
impl<'a> Plan<'a> {
    pub fn acquire(
        graph: &'a Package,
        fuel: &mut usize,
        mut classify: impl FnMut(&Term, &mut usize) -> Result<Delivery, String>,
    ) -> Result<Self, String> {
        graph.validate(fuel)?;
        let links = TypeLinks::new(graph, fuel)?;
        let mut entries: Vec<Entry> = Vec::new();
        for function in &graph.functions {
            visit::expressions(&function.function, fuel, |expr, fuel| {
                if let ExprKind::EffectHandle { handlers, .. } = &expr.kind {
                    for handler in handlers.iter().filter(|h| h.abi == Abi::DirectPassing) {
                        let Term::Function { result, .. } = &handler.signature else {
                            return Err("effect value signature is not callable".into());
                        };
                        graph.validate_type(&links, result, fuel)?;
                        for argument in &handler.operation.arguments {
                            graph.validate_type(&links, argument, fuel)?;
                        }
                        let mut found = false;
                        for entry in &entries {
                            *fuel = fuel
                                .checked_sub(1)
                                .ok_or("effect value plan budget exhausted")?;
                            for argument in &entry.operation.arguments {
                                graph.validate_type(&links, argument, fuel)?;
                            }
                            if entry.operation == handler.operation {
                                if &entry.result != result.as_ref() {
                                    return Err("effect value occurrence result changed".into());
                                }
                                found = true;
                                break;
                            }
                        }
                        if !found {
                            let delivery = classify(result, fuel)?;
                            entries.push(Entry {
                                operation: handler.operation.clone(),
                                result: result.as_ref().clone(),
                                delivery,
                            });
                        }
                    }
                }
                Ok(())
            })?;
        }
        Ok(Self {
            graph,
            links,
            entries,
        })
    }
    pub fn belongs_to(&self, graph: &Package) -> bool {
        std::ptr::eq(self.graph, graph)
    }
    pub fn delivery(
        &self,
        operation: &Operation,
        result: &Term,
        fuel: &mut usize,
    ) -> Result<Delivery, String> {
        self.graph.validate_type(&self.links, result, fuel)?;
        for argument in &operation.arguments {
            self.graph.validate_type(&self.links, argument, fuel)?;
        }
        for entry in &self.entries {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("effect value plan budget exhausted")?;
            self.graph.validate_type(&self.links, &entry.result, fuel)?;
            for argument in &entry.operation.arguments {
                self.graph.validate_type(&self.links, argument, fuel)?;
            }
            if &entry.operation == operation && &entry.result == result {
                return Ok(entry.delivery);
            }
        }
        Err("effect installation has no acquired value delivery".into())
    }
}
impl fmt::Display for Plan<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(effect-value-plan (roots")?;
        for root in &self.graph.roots {
            write!(f, " {root}")?;
        }
        f.write_str(") (entries")?;
        for entry in &self.entries {
            write!(
                f,
                " (entry {} (result {}) (delivery {}))",
                entry.operation, entry.result, entry.delivery
            )?;
        }
        f.write_str("))")
    }
}
