use crate::hir::{Hir, HirStruct, ResolvedType};

fn format_type_args(hir: &Hir, args: &[ResolvedType]) -> String {
    if args.is_empty() {
        String::new()
    } else {
        let names: Vec<String> = args.iter().map(|t| name_type(hir, t, &[])).collect();
        format!("<{}>", names.join(","))
    }
}

pub fn name_struct(hir: &Hir, s: &HirStruct, args: &[ResolvedType]) -> String {
    let module = &hir.modules[&s.module];
    format!("{}::{}{}", module.name, s.name, format_type_args(hir, args))
}

pub fn name_function(hir: &Hir, fn_id: crate::hir::FnId, args: &[ResolvedType]) -> String {
    let f = &hir.functions[&fn_id];
    let module = &hir.modules[&f.module];
    // Methods need the owner struct in the symbol so e.g. `TcpListener.close`
    // and `TcpStream.close` don't collide at link time.
    match f.owner {
        Some(owner_id) => {
            let owner = &hir.structs[&owner_id];
            format!(
                "{}::{}::{}{}",
                module.name,
                owner.name,
                f.name,
                format_type_args(hir, args),
            )
        }
        None => format!("{}::{}{}", module.name, f.name, format_type_args(hir, args)),
    }
}

pub fn name_interface(hir: &Hir, ty: &ResolvedType, args: &[ResolvedType]) -> String {
    match ty {
        ResolvedType::Interface(hir_id, _) => {
            let i = &hir.interfaces[hir_id];
            let module = &hir.modules[&i.module];
            format!("{}::{}{}", module.name, i.name, format_type_args(hir, args))
        }
        _ => panic!("Expected an interface type"),
    }
}

pub fn name_type(hir: &Hir, ty: &ResolvedType, _args: &[ResolvedType]) -> String {
    match ty {
        ResolvedType::Struct(hir_id, struct_args) => {
            name_struct(hir, &hir.structs[hir_id], struct_args)
        }
        ResolvedType::Union(types) => {
            let variant_names: Vec<String> = types.iter().map(|t| name_type(hir, t, &[])).collect();
            format!("Union<{}>", variant_names.join("|"))
        }
        ResolvedType::Primitive(ty) => format!("{:?}", ty),
        ResolvedType::Null => "Null".to_string(),
        ResolvedType::Function(args, ret) => {
            let arg_names: Vec<String> = args.iter().map(|t| name_type(hir, t, &[])).collect();
            let ret_name = name_type(hir, ret, &[]);
            format!("Function<{}, {}>", arg_names.join(","), ret_name)
        }
        ResolvedType::Interface(_, iface_args) => name_interface(hir, ty, iface_args),
        ResolvedType::TypeParam(_) => panic!("TypeParam should not be named directly"),
    }
}
