//! Private, completed data updates. Ordinary place elaboration chooses field
//! paths and checks bit writes; invocation performs the update in a fresh frame.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn assign_generator_place(&mut self,target:&Expr<'_>,value:&Expr<'_>)->Result<(),Error> {
        let mut root=target;
        loop {
            self.required_steps=self.required_steps.checked_sub(1)
                .ok_or_else(||error(target.span,"generator place traversal budget exhausted"))?;
            match root.kind {
                H::Projection(inner,_) | H::Index(inner,_) | H::TakeBitVec(inner)=>root=inner,
                H::ResolvedLocal(..) | H::ResolvedFnParam(..)=>break,
                _=>return Err(error(target.span,"generator assignment requires a direct owned data place; pointer writes are not supported")),
            }
        }
        let id=binding(root)?;
        if !self.generator_data.contains_key(&id) {
            return Err(error(target.span,"generator place assignment cannot modify runtime context or code storage"));
        }
        let ty=self.locals.get(&id).cloned()
            .ok_or_else(||error(target.span,"generator place root has no type"))?;
        // Leave a typed local placeholder while elaborating both sides. The
        // right side may read the same object, and must observe its old value.
        let previous=self.generator_data.insert(id,None).unwrap();
        let mode=std::mem::replace(&mut self.in_required_comptime,true);
        let result=(|| {
            let assignment=Expr {kind:H::Assign(target,value),..target.clone()};
            let assignment=self.expr(&assignment,Some(&p(P::Void)))?;
            self.validate_generator_data(&assignment,target.span)?;
            if self.checking_code_generator {return Ok(None);}
            let previous=previous.as_ref().ok_or_else(||error(target.span,"generator data place is not completed"))?;
            constants::charge_copy(previous,&mut self.required_steps,target.span)?;
            let body=n::Expr {ty:ty.clone(),source:source(target.span),kind:K::Block {
                statements:vec![n::Expr {ty:p(P::Void),source:source(target.span),kind:K::Let {
                    binding:id,value:Box::new(previous.clone()),
                }},assignment],
                result:Some(Box::new(n::Expr {ty:ty.clone(),source:source(target.span),kind:K::Local(id)})),
            }};
            let (updated,ty)=self.required_normal_input(target,body,vec![])?;
            self.materialize_generator_data(updated,ty,target.span).map(Some)
        })();
        self.in_required_comptime=mode;
        match result {
            Ok(updated)=>{self.generator_data.insert(id,updated);Ok(())},
            Err(failure)=>{self.generator_data.insert(id,previous);Err(failure)},
        }
    }
}
