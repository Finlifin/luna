//! Lexical code and binding handoff for declaration input producers.
use crate::{CompilationSession, DeclarationInputPlan, Error, Program, Register};
use nir::program::{Expr, LocalId, NormalFormPreparation, SourceRange};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::{Arc, Weak},
};

/// One lexical producer and the bindings visible after normal completion.
/// Register numbers belong exclusively to this program.
#[derive(Clone, Debug)]
pub struct PreparationProgram {
    pub program: Arc<Program>,
    pub exports: Arc<BoundExports>,
}

impl PreparationProgram {
    fn weight(&self) -> usize {
        1 + self.program.storage_work()
            + self.program.locals().len()
            + self.program.registers()
            + self.exports.bindings.len()
    }
}

impl fmt::Display for PreparationProgram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(preparation-program {} {})", self.exports, self.program)
    }
}

#[derive(Debug)]
pub struct BoundExports {
    source: SourceRange,
    bindings: Vec<(LocalId, Register)>,
}

impl BoundExports {
    pub fn source(&self) -> SourceRange {
        self.source
    }
    pub fn bindings(&self) -> &[(LocalId, Register)] {
        &self.bindings
    }
}

impl fmt::Display for BoundExports {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(bound-exports (source {} {}) (bindings",
            self.source.start, self.source.end
        )?;
        for (binding, slot) in &self.bindings {
            write!(f, " ({binding} %{slot})")?;
        }
        f.write_str("))")
    }
}

#[derive(Clone, Copy)]
pub struct PreparationSite<'a> {
    source: &'a NormalFormPreparation,
    owner: Option<(&'a Arc<DeclarationInputPlan>, usize)>,
}

impl<'a> PreparationSite<'a> {
    pub fn borrowed(source: &'a NormalFormPreparation) -> Self {
        Self {
            source,
            owner: None,
        }
    }

    pub(super) fn owned(
        source: &'a NormalFormPreparation,
        owner: &'a Arc<DeclarationInputPlan>,
        index: usize,
    ) -> Self {
        Self {
            source,
            owner: Some((owner, index)),
        }
    }

    pub fn expression(self) -> &'a Expr {
        &self.source.expression
    }

    pub fn compiled(self, session: &CompilationSession) -> Option<PreparationProgram> {
        let (owner, index) = self.owner?;
        session.preparations.get(owner, index)
    }

    /// Lower at the source position in the current code world. Only successful
    /// programs enter the session cache. Anonymous NIR borrows have no stable key.
    pub fn compile(
        self,
        session: &mut CompilationSession,
        fuel: &mut usize,
    ) -> Result<PreparationProgram, Error> {
        let expression = self.expression();
        let source = expression.source;
        let mut names = BTreeSet::new();
        let mut unique = true;
        let valid_pattern = self.source.visit_bindings(
            |count| {
                *fuel = fuel.checked_sub(count).ok_or(Error::Budget(source))?;
                Ok(())
            },
            |binding| unique &= names.insert(binding),
        )?;
        if !valid_pattern || !unique {
            return Err(Error::InvalidPreparation(source));
        }
        let program = session.compile_lexical_expression(expression, fuel)?;
        let mut exports = Vec::with_capacity(names.len());
        for name in names {
            let index = program
                .locals()
                .binary_search_by_key(&name, |(id, _)| *id)
                .map_err(|_| Error::InvalidPreparation(source))?;
            exports.push(program.locals()[index]);
        }
        let fragment = PreparationProgram {
            program: Arc::new(program),
            exports: Arc::new(BoundExports {
                bindings: exports,
                source,
            }),
        };
        if let Some((owner, index)) = self.owner {
            session.preparations.insert(owner, index, fragment.clone());
        }
        Ok(fragment)
    }
}

#[derive(Default)]
pub(super) struct Cache {
    entries: BTreeMap<(usize, usize), Entry>,
    weight: usize,
}

struct Entry {
    // Reserve the key's allocation identity without retaining its source plan.
    _owner: Weak<DeclarationInputPlan>,
    fragment: PreparationProgram,
}

impl Cache {
    fn get(&self, owner: &Arc<DeclarationInputPlan>, index: usize) -> Option<PreparationProgram> {
        let entry = self.entries.get(&(Arc::as_ptr(owner) as usize, index))?;
        Some(entry.fragment.clone())
    }

    fn insert(
        &mut self,
        owner: &Arc<DeclarationInputPlan>,
        index: usize,
        fragment: PreparationProgram,
    ) {
        const LIMIT: usize = 100_000;
        let weight = fragment.weight();
        if weight > LIMIT {
            return;
        }
        if self.weight + weight > LIMIT {
            self.entries.clear();
            self.weight = 0;
        }
        let key = (Arc::as_ptr(owner) as usize, index);
        if let Some(previous) = self.entries.insert(
            key,
            Entry {
                _owner: Arc::downgrade(owner),
                fragment,
            },
        ) {
            self.weight -= previous.fragment.weight();
        }
        self.weight += weight;
    }
}
