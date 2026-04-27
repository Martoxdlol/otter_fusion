use std::collections::HashMap;

pub struct MirFnId(pub u32);

pub struct MirProgram {
    pub types: HashMap<String, MirType>,

    pub functions: HashMap<MirFnId, MirFunction>,
}

pub struct MirFunction {
    // name

    // parameters

    // return type

    // body (MIR statements)
}

pub enum MirType {
    // struct
    // union
    // function (en una mejor vida)
}
