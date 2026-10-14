//! Match data once in the ordinary interpreter, exporting the selected arm's
//! bindings through a private nominal packet. No code witness enters the VM.
use super::*;

struct Arm {
    pattern:n::Pattern,
    bindings:Vec<(n::LocalId,Term)>,
}

impl Checker<'_, '_> {
    pub(in crate::program::cmtt) fn generator_code_match(&mut self,site:&Expr<'_>,subject:&Expr<'_>,
        arms:&[hir::pattern::PatternArm<'_>],interface:Option<&Interface>,depth:usize)
        ->Result<Arc<ExpressionCode>,Error> {
        let codes=self.generator_match(site,subject,arms,None,|this,_,body|
            this.code_value(body,interface,depth+1))?;
        let mut result:Option<Arc<ExpressionCode>>=None;
        for code in codes {
            if let Some(first)=&result {
                self.require_expression(&code.result,&first.result,site.span)?;
                if code.context!=first.context || code.ports!=first.ports {
                    let expected=Interface {infer_context:false,context:first.context.clone(),ports:first.ports.clone(),
                        defaults:first.defaults.clone(),result:first.result.clone()};
                    self.ascribe_code(code,Some(&expected),site.span)?;
                }
            } else {result=Some(code);}
        }
        result.ok_or_else(||error(site.span,"generator match requires a code-producing arm"))
    }

    pub(super) fn generator_match<T>(&mut self,site:&Expr<'_>,subject:&Expr<'_>,
        arms:&[hir::pattern::PatternArm<'_>],expected:Option<&Term>,
        mut body:impl FnMut(&mut Self,usize,&Expr<'_>)->Result<T,Error>)
        ->Result<Vec<T>,Error> {
        let value=if self.checking_code_generator {
            let value=self.expr(subject,expected)?;
            self.validate_generator_data(&value,subject.span)?;
            value
        } else {
            let (value,ty)=self.required_typed_expression(subject,expected)?;
            self.materialize_generator_data(value,ty,subject.span)?
        };
        let mut checked=Vec::new();
        let mut results=Vec::new();
        for (index,arm) in arms.iter().enumerate() {
            let (prepared,code)=self.generator_scope(arm.span,|this| {
                this.immutable.extend(pattern_binding_ids(&arm.pat));
                let pattern=this.match_arm_pattern(arm,&value)?;
                let bindings=pattern_bindings(&arm.pat)?.into_values().map(|id| {
                    this.locals.get(&id).cloned().map(|ty|(id,ty))
                        .ok_or_else(||error(arm.span,"generator match did not establish its binding"))
                }).collect::<Result<Vec<_>,_>>()?;
                let code=if this.checking_code_generator {
                    for (id,_) in &bindings {this.generator_data.insert(*id,None);}
                    Some(this.under_matched_pattern_type_facts(&pattern,&value,arm.span,|this|
                        body(this,index,arm.body))?)
                } else {None};
                constants::charge_copy(&value,&mut this.required_steps,subject.span)?;
                // Probe only phase/dependency legality, without running a guard.
                let probe=n::Expr {ty:p(P::Bool),source:source(arm.span),kind:K::Match {
                    value:Box::new(value.clone()),arms:vec![n::MatchArm {
                        pattern,guard:None,
                        body:n::Expr {ty:p(P::Bool),source:source(arm.span),kind:K::Literal(n::Literal::Bool(true))},
                    }],
                }};
                this.validate_generator_data(&probe,arm.span)?;
                let K::Match {mut arms,..}=probe.kind else {unreachable!()};
                let pattern=arms.pop().unwrap().pattern;
                Ok((Arm {pattern,bindings},code))
            })?;
            if let Some(code)=code {results.push(code);}
            checked.push(prepared);
        }
        let patterns=checked.iter().map(|arm|&arm.pattern).collect::<Vec<_>>();
        if self.patterns_exhaustive(&patterns,&value.ty)!=crate::coverage::Coverage::Exhaustive {
            return Err(error(site.span,"non-exhaustive generator match or exhausted coverage budget; add an unguarded fallback"));
        }
        if self.checking_code_generator {
            return Ok(results);
        }
        let declaration=DeclarationId {package:u64::MAX-2,local:self.next_temporary_binding};
        self.next_temporary_binding=self.next_temporary_binding.checked_add(1)
            .ok_or_else(||error(site.span,"generator match identity space exhausted"))?;
        if self.enums.contains_key(&declaration) {return Err(error(site.span,"generator match packet identity collision"));}
        let packet_type=Term::Nominal {declaration,arguments:vec![]};
        let mut fuel=usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before=fuel;
        let bounded=(|| {
            for arm in &checked {for (_,ty) in &arm.bindings {nir::type_computations::charge_term(ty,&mut fuel)?;}}
            Ok::<_,nir::type_computations::Error>(())
        })();
        self.required_steps-=(before-fuel) as u64;
        bounded.map_err(|failure|error(site.span,failure.to_string()))?;
        constants::charge_copy(&value,&mut self.required_steps,site.span)?;
        let variants=checked.iter().enumerate().map(|(index,arm)|n::Variant {
            name:format!("arm_{index}"),shape:n::VariantShape::Tuple,
            fields:arm.bindings.iter().map(|(_,ty)|ty.clone()).collect(),
        }).collect();
        self.enums.insert(declaration,n::Enum {declaration,name:"<generator-match-packet>".into(),
            type_parameters:vec![],capabilities:vec![],requirements:vec![],variants,groups:vec![],encoding:None,symbol_encoding:None});
        self.code_packet_nominals.insert(declaration);
        let mut exports=Vec::with_capacity(checked.len());
        let selection=(|| {
            let cases=checked.into_iter().enumerate().map(|(index,arm)| {
                let fields=arm.bindings.iter().map(|(id,ty)|n::Expr {
                    ty:ty.clone(),source:source(arms[index].span),kind:K::Local(*id),
                }).collect();
                exports.push(arm.bindings);
                n::MatchArm {pattern:arm.pattern,guard:None,
                    body:n::Expr {ty:packet_type.clone(),source:source(arms[index].span),kind:K::Variant {declaration,index,fields}},
                }
            }).collect();
            let root=n::Expr {ty:packet_type,source:source(site.span),kind:K::Match {value:Box::new(value.clone()),arms:cases}};
            self.required_normal_input(subject,root,vec![])
        })();
        self.code_packet_nominals.remove(&declaration);
        self.enums.remove(&declaration);
        let (selected,_)=selection?;
        let comptime_interpretor::Value::Variant {declaration:actual,index,fields,arguments}=selected else {
            return Err(error(site.span,"generator match produced an invalid binding packet"));
        };
        if actual!=declaration || !arguments.is_empty() || index>=exports.len()
            || fields.len()!=exports[index].len() {
            return Err(error(site.span,"generator match packet disagrees with its checked interface"));
        }
        let selected=self.generator_scope(arms[index].span,|this| {
            // Re-establish type facts, not matching effects: elaborating a
            // guard builds IR here but never invokes it a second time.
            let pattern=this.match_arm_pattern(&arms[index],&value)?;
            for ((id,ty),value) in exports[index].iter().zip(fields) {
                let value=this.materialize_generator_data(value,ty.clone(),arms[index].span)?;
                this.locals.insert(*id,ty.clone());
                this.generator_data.insert(*id,Some(value));
                this.immutable.insert(*id);
            }
            this.under_matched_pattern_type_facts(&pattern,&value,arms[index].span,|this|
                body(this,index,arms[index].body))
        })?;
        Ok(vec![selected])
    }
}
