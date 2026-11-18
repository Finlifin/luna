//! Inspect the result of checked Type-producing code without running it.
use super::*;

struct Observer<'a> {
    package: &'a n::Package,
    head: DeclarationId,
    active: BTreeSet<DeclarationId>,
    results: BTreeMap<DeclarationId, bool>,
    fuel: &'a mut usize,
}

pub(super) fn guarantees(
    package: &n::Package,
    entry: DeclarationId,
    head: DeclarationId,
    fuel: &mut usize,
) -> Result<bool, String> {
    Observer {
        package,
        head,
        active: BTreeSet::new(),
        results: BTreeMap::new(),
        fuel,
    }
    .function(entry, 0)
}

impl Observer<'_> {
    fn function(&mut self, id: DeclarationId, depth: usize) -> Result<bool, String> {
        if depth >= 64 || self.active.contains(&id) {
            return Ok(false);
        }
        if let Some(&answer) = self.results.get(&id) {
            return Ok(answer);
        }
        *self.fuel = self
            .fuel
            .checked_sub(self.package.functions.len())
            .ok_or("result head inspection budget exhausted")?;
        let Some(function) = self
            .package
            .functions
            .iter()
            .find(|f| f.declaration == id && f.phase == n::FunctionPhase::Comptime)
        else {
            return Ok(false);
        };
        self.active.insert(id);
        // A tail expression describes only fallthrough. An early return,
        // including one inside a statement, must give the same guarantee.
        let mut exits = Vec::new();
        nir::visit::expression(&function.body, self.fuel, |expr, _| {
            if let K::Return(value) = &expr.kind {
                exits.push(value.as_deref());
            }
            Ok(())
        })?;
        let mut answer = self.terminal(&function.body, depth + 1)?;
        for exit in exits {
            if !answer {
                break;
            }
            answer = match exit {
                Some(value) => self.terminal(value, depth + 1)?,
                None => false,
            };
        }
        self.active.remove(&id);
        self.results.insert(id, answer);
        Ok(answer)
    }

    fn terminal(&mut self, value: &n::Expr, depth: usize) -> Result<bool, String> {
        *self.fuel = self
            .fuel
            .checked_sub(1)
            .ok_or("result head inspection budget exhausted")?;
        if depth >= 96 {
            return Ok(false);
        }
        match &value.kind {
            K::Block {
                result: Some(value),
                ..
            }
            | K::CallPreparation {
                result: Some(value),
                ..
            }
            | K::Return(Some(value)) => self.terminal(value, depth + 1),
            K::If {
                then_branch,
                else_branch: Some(no),
                ..
            } => Ok(self.terminal(then_branch, depth + 1)? && self.terminal(no, depth + 1)?),
            K::NormalFormType { head, .. } | K::TypeValue(Term::NormalForm { head, .. }) => {
                Ok(self.head == *head)
            }
            K::Call { callee, .. } => match &callee.kind {
                K::Function(id)
                | K::FunctionInstance {
                    declaration: id, ..
                } => self.function(*id, depth + 1),
                _ => Ok(false),
            },
            _ => Ok(false),
        }
    }
}
