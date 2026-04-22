```flurry
-- cabi.cstruct: comptime fn(...children: meta.List<Void>, ...properties: meta.Map<Symbol, Type>) -> Type
newtype Node = cabi.cstruct {
    data: cabi.int,
    next: cabi.Ptr<Node>,
}

newtype Enemy = ecs.component {
    health: u32,
    damage: u32,
}

newtype User = database.entity {
    id: database.Uuid,
    name: String,
    email: String,
}

newtype MyJuliaStruct = julia.jlstruct {
    field1: f64,
    field2: String,
}

newtype MyKotlinClass = kotlin.class' {
    generics: [T, U],
    
    id: kotlin.Int,
    name: kotlin.String,
    ...
}
```