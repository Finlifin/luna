//! Control of the comptime construction program. Runtime control inside a
//! quote remains ordinary NIR and never targets these construction loops.
use super::*;

pub(super) enum Control {
    Next,
    Break(n::LocalId),
    Continue(n::LocalId),
    Return(Arc<ExpressionCode>),
    Abstract(Flow),
}

#[derive(Default)]
pub(super) struct Flow {
    pub(super) next:bool,
    breaks:BTreeSet<n::LocalId>,
    continues:BTreeSet<n::LocalId>,
    pub(super) returns:Vec<Arc<ExpressionCode>>,
}

impl Control {
    pub(super) fn flow(self)->Flow {
        match self {
            Self::Next=>Flow {next:true,..Default::default()},
            Self::Break(target)=>Flow {breaks:BTreeSet::from([target]),..Default::default()},
            Self::Continue(target)=>Flow {continues:BTreeSet::from([target]),..Default::default()},
            Self::Return(code)=>Flow {returns:vec![code],..Default::default()},
            Self::Abstract(flow)=>flow,
        }
    }
}
impl Flow {
    pub(super) fn has_transfer(&self)->bool {!self.breaks.is_empty() || !self.continues.is_empty()}
}
impl fmt::Display for Flow {
    fn fmt(&self,f:&mut fmt::Formatter<'_>)->fmt::Result {
        write!(f,"(generator-flow (next {}) (breaks",self.next)?;
        for target in &self.breaks {write!(f," {target}")?;}
        f.write_str(") (continues")?;
        for target in &self.continues {write!(f," {target}")?;}
        f.write_str(") (returns")?;
        for code in &self.returns {write!(f," {code}")?;}
        f.write_str("))")
    }
}

fn known_boolean(value:&n::Expr,depth:usize)->Option<bool> {
    if depth>=64 {return None;}
    match &value.kind {
        K::Literal(n::Literal::Bool(value))=>Some(*value),
        K::Not(value)=>Some(!known_boolean(value,depth+1)?),
        K::And(left,right)=>Some(known_boolean(left,depth+1)? && known_boolean(right,depth+1)?),
        K::Or(left,right)=>Some(known_boolean(left,depth+1)? || known_boolean(right,depth+1)?),
        _=>None,
    }
}

impl Checker<'_, '_> {
    fn join_generator_flows(&mut self,mut left:Flow,right:Flow,span:Span)->Result<Flow,Error> {
        let work=left.breaks.len().saturating_add(left.continues.len()).saturating_add(left.returns.len())
            .saturating_add(right.breaks.len()).saturating_add(right.continues.len()).saturating_add(right.returns.len());
        self.required_steps=self.required_steps.checked_sub(work as u64)
            .ok_or_else(||error(span,"generator control-flow budget exhausted"))?;
        left.next|=right.next;
        left.breaks.extend(right.breaks);
        left.continues.extend(right.continues);
        left.returns.extend(right.returns);
        Ok(left)
    }

    pub(super) fn sequence_generator_control(&mut self,previous:Control,next:Control,span:Span)->Result<Control,Error> {
        let mut previous=previous.flow();
        if previous.next {
            previous.next=false;
            previous=self.join_generator_flows(previous,next.flow(),span)?;
        }
        Ok(Control::Abstract(previous))
    }
    pub(super) fn generator_statement(&mut self, value:&Expr<'_>, interface:Option<&Interface>,
        depth:usize, loops:&mut Vec<n::LocalId>) -> Result<Control,Error> {
        if depth>=64 {return Err(error(value.span,"generator statement nesting depth exhausted"));}
        self.required_steps=self.required_steps.checked_sub(1)
            .ok_or_else(||error(value.span,"generator construction step budget exhausted"))?;
        match &value.kind {
            H::Semi(inner)=>self.generator_statement(inner,interface,depth+1,loops),
            H::Let(decl) if decl.otherwise.is_some()=>self.generator_conditional_binding(value,decl,interface,depth+1,loops),
            H::Let(decl)=>{self.generator_binding(decl)?;Ok(Control::Next)},
            H::Assign(target,right)=>{self.assign_generator_binding(target,right,depth+1)?;Ok(Control::Next)},
            H::AssignOp(op,target,right)=>{
                let mut root=*target;
                while let H::Projection(inner,_) = root.kind {root=inner;}
                if !matches!(root.kind,H::ResolvedLocal(..)|H::ResolvedFnParam(..)) {
                    return Err(error(target.span,"generator compound assignment currently requires a binding or direct field path"));
                }
                // Ordinary binary elaboration supplies operator protocols and
                // arithmetic checks; this layer only owns the final local write.
                let expression=Expr {kind:H::Binary(*op,target,right),..value.clone()};
                self.assign_generator_binding(target,&expression,depth+1)?;
                Ok(Control::Next)
            }
            H::Block(block)=>self.generator_statement_block(block,interface,depth+1,loops),
            H::If(condition,yes,no)=>{
                if self.checking_code_generator {
                    let condition_ir=self.expr(condition,Some(&p(P::Bool)))?;
                    self.validate_generator_data(&condition_ir,condition.span)?;
                    let yes=self.generator_statement_block(yes,interface,depth+1,loops)?;
                    let no=if let Some(no)=no {self.generator_statement(no,interface,depth+1,loops)?} else {Control::Next};
                    match known_boolean(&condition_ir,0) {
                        Some(true)=>Ok(yes),Some(false)=>Ok(no),
                        None=>Ok(Control::Abstract(self.join_generator_flows(yes.flow(),no.flow(),value.span)?)),
                    }
                } else {
                    let (condition_value,_)=self.required_typed_expression(condition,Some(&p(P::Bool)))?;
                    match condition_value {
                        comptime_interpretor::Value::Bool(true)=>self.generator_statement_block(yes,interface,depth+1,loops),
                        comptime_interpretor::Value::Bool(false)=>match no {
                            Some(no)=>self.generator_statement(no,interface,depth+1,loops),
                            None=>Ok(Control::Next),
                        },
                        _=>Err(error(condition.span,"generator branch requires a comptime boolean")),
                    }
                }
            }
            H::Match(subject,arms)=>{
                let controls=self.generator_match(value,subject,arms,None,|this,_,body|
                    this.generator_statement(body,interface,depth+1,loops))?;
                if self.checking_code_generator {
                    let mut flow=Flow::default();
                    for control in controls {
                        flow=self.join_generator_flows(flow,control.flow(),value.span)?;
                    }
                    Ok(Control::Abstract(flow))
                } else {
                    controls.into_iter().next().ok_or_else(||error(value.span,"generator match selected no arm"))
                }
            }
            H::Loop(body)=>{
                let identity=local(value.hir_id);
                loops.push(identity);
                let result=(|| {
                    if self.checking_code_generator {
                        let mut flow=self.generator_statement_block(body,interface,depth+1,loops)?.flow();
                        // Backedges do not finish the enclosing block. Only a
                        // break targeting this loop reaches its successor.
                        flow.next=flow.breaks.remove(&identity);
                        flow.continues.remove(&identity);
                        return Ok(Control::Abstract(flow));
                    }
                    loop {
                        self.required_steps=self.required_steps.checked_sub(1)
                            .ok_or_else(||error(value.span,"generator construction step budget exhausted"))?;
                        match self.generator_statement_block(body,interface,depth+1,loops)? {
                            Control::Next=>{},
                            Control::Continue(target) if target==identity=>{},
                            Control::Break(target) if target==identity=>return Ok(Control::Next),
                            transfer=>return Ok(transfer),
                        }
                    }
                })();
                loops.pop();
                result
            }
            H::BreakTo(id)|H::ContinueTo(id)=>{
                let target=local(*id);
                if !loops.contains(&target) {return Err(error(value.span,"generator loop transfer targets an unavailable loop"));}
                Ok(if matches!(value.kind,H::BreakTo(_)) {Control::Break(target)} else {Control::Continue(target)})
            }
            H::Break(label)|H::Continue(label)=>{
                if label.name!=hir::Symbol::invalid() {return Err(error(value.span,"unresolved generator loop label"));}
                let target=*loops.last().ok_or_else(||error(value.span,"generator loop transfer outside a loop"))?;
                Ok(if matches!(value.kind,H::Break(_)) {Control::Break(target)} else {Control::Continue(target)})
            }
            H::Return(Some(code))=>Ok(Control::Return(self.code_value(code,interface,depth+1)?)),
            H::Return(None)=>Err(error(value.span,"code generator return requires a code value")),
            _ if self.code_producer(value)=>{
                self.code_value(value,None,depth+1)?;
                Ok(Control::Next)
            }
            _=>{
                if self.checking_code_generator {
                    let checked=self.expr(value,None)?;
                    self.validate_generator_data(&checked,value.span)?;
                } else {self.required_typed_expression(value,None)?;}
                Ok(Control::Next)
            }
        }
    }

    fn generator_conditional_binding(&mut self,site:&Expr<'_>,decl:&hir::decl::LetDecl<'_>,
        interface:Option<&Interface>,depth:usize,loops:&mut Vec<n::LocalId>)->Result<Control,Error> {
        let initializer=decl.init.ok_or_else(||error(decl.span,"generator binding needs an initializer"))?;
        if self.code_producer(initializer) {
            return Err(error(decl.span,"conditional destructuring requires generator data, not a code witness"));
        }
        let expected=decl.ty.map(|ty|self.type_expr(ty)).transpose()?;
        let otherwise=decl.otherwise.unwrap();
        let arms=[
            hir::pattern::PatternArm {hir_id:decl.hir_id,pat:decl.pattern.clone(),guard:decl.guard,body:initializer,span:decl.span},
            hir::pattern::PatternArm {hir_id:decl.hir_id,
                pat:hir::pattern::Pattern {hir_id:decl.hir_id,kind:hir::pattern::PatternKind::Wild,span:decl.span},
                guard:None,body:otherwise,span:decl.span},
        ];
        let outputs=self.generator_match(site,initializer,&arms,expected.as_ref(),|this,index,body| {
            if index==1 {
                let control=this.generator_statement(body,interface,depth+1,loops)?;
                let falls_through=match &control {Control::Next=>true,Control::Abstract(flow)=>flow.next,_=>false};
                if falls_through {return Err(error(body.span,"generator let-else branch must diverge"));}
                return Ok((control,Vec::new()));
            }
            let mut bindings=Vec::new();
            for id in pattern_bindings(&decl.pattern)?.into_values() {
                let ty=this.locals.get(&id).ok_or_else(||error(decl.span,"generator let-else missing binding type"))?.clone();
                let data=this.generator_data.get(&id).ok_or_else(||error(decl.span,"generator let-else missing binding value"))?;
                if let Some(data)=data {constants::charge_copy(data,&mut this.required_steps,decl.span)?;}
                bindings.push((id,ty,data.clone()));
            }
            Ok((Control::Next,bindings))
        })?;
        let mut result=Control::Next;
        let mut flow=Flow::default();
        for (control,bindings) in outputs {
            for (id,ty,value) in bindings {
                self.locals.insert(id,ty);
                self.generator_data.insert(id,value);
                if decl.mutable {self.immutable.remove(&id);} else {self.immutable.insert(id);}
            }
            if self.checking_code_generator {flow=self.join_generator_flows(flow,control.flow(),decl.span)?;}
            else {result=control;}
        }
        Ok(if self.checking_code_generator {Control::Abstract(flow)} else {result})
    }

    fn generator_statement_block(&mut self, block:&hir::Block<'_>, interface:Option<&Interface>,
        depth:usize, loops:&mut Vec<n::LocalId>)->Result<Control,Error> {
        self.generator_scope(block.span,|this| {
            let mut result=Control::Next;
            for value in block.stmts.iter().chain(block.expr.into_iter()) {
                let control=this.generator_statement(value,interface,depth+1,loops)?;
                if this.checking_code_generator {
                    result=this.sequence_generator_control(result,control,value.span)?;
                } else if !matches!(control,Control::Next) {
                    return Ok(control);
                }
            }
            Ok(result)
        })
    }
}
