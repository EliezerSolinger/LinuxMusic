//! The symbol table: the variables, constants and host functions an expression may name. Built once on the main
//! thread; every [`VarId`] and [`FnId`] it hands out is valid in every [`crate::Expr`] compiled against it (the
//! ids are plain indexes, in registration order)

use crate::builtin;
use crate::error::SymbolError;

/// A variable of the table: the index of its slot in the `vars` slice of `eval`
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VarId(pub u32);

/// A host function of the table: what [`crate::Host::call`] receives
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FnId(pub u32);

impl VarId {
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl FnId {
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A host function's declaration
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnDef {
    pub name: String,
    /// 0 to 8 arguments
    pub arity: u8,
    /// `rand` and `integrate`: the call changes the host's state, so the compiler never folds, reorders or drops
    /// it by an optimisation, and [`crate::Expr::has_side_effects`] reports it
    pub side_effects: bool,
}

pub(crate) const MAX_ARITY: u8 = 8;

/// The names an expression may use. `pi` and `e` are registered from the start
#[derive(Clone, Debug)]
pub struct Symbols {
    /// Lowercase names (exprtk's symbol tables compare case-insensitively)
    vars: Vec<String>,
    funcs: Vec<FnDef>,
    consts: Vec<(String, f32)>,
}

impl Default for Symbols {
    fn default() -> Self {
        Self::new()
    }
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => chars.all(|c| c.is_ascii_alphanumeric() || c == '_'),
        _ => false,
    }
}

impl Symbols {
    pub fn new() -> Self {
        Self {
            vars: Vec::new(),
            funcs: Vec::new(),
            consts: vec![("pi".to_owned(), crate::ops::CONST_PI), ("e".to_owned(), crate::ops::CONST_E)],
        }
    }

    fn check_name(&self, name: &str) -> Result<String, SymbolError> {
        if !valid_name(name) {
            return Err(SymbolError::Invalid);
        }
        let lower = name.to_ascii_lowercase();
        if builtin::is_reserved(&lower) {
            return Err(SymbolError::Reserved);
        }
        if self.vars.contains(&lower)
            || self.consts.iter().any(|(n, _)| *n == lower)
            || self.funcs.iter().any(|f| f.name == lower)
        {
            return Err(SymbolError::Duplicate);
        }
        Ok(lower)
    }

    /// Registers a variable the host writes before every evaluation. The id is its index in `vars`
    pub fn add_var(&mut self, name: &str) -> Result<VarId, SymbolError> {
        let lower = self.check_name(name)?;
        let id = u32::try_from(self.vars.len()).map_err(|_| SymbolError::Full)?;
        self.vars.push(lower);
        Ok(VarId(id))
    }

    /// Registers a constant, folded into the expressions at compile time
    pub fn add_const(&mut self, name: &str, value: f32) -> Result<(), SymbolError> {
        let lower = self.check_name(name)?;
        self.consts.push((lower, value));
        Ok(())
    }

    /// Registers a host function of `arity` arguments (0 to 8). Calls reach [`crate::Host::call`]
    pub fn add_fn(&mut self, name: &str, arity: u8, side_effects: bool) -> Result<FnId, SymbolError> {
        if arity > MAX_ARITY {
            return Err(SymbolError::Invalid);
        }
        let lower = self.check_name(name)?;
        let id = u32::try_from(self.funcs.len()).map_err(|_| SymbolError::Full)?;
        self.funcs.push(FnDef { name: lower, arity, side_effects });
        Ok(FnId(id))
    }

    pub fn var(&self, name: &str) -> Option<VarId> {
        let lower = name.to_ascii_lowercase();
        self.vars.iter().position(|n| *n == lower).map(|i| VarId(i as u32))
    }

    pub fn function(&self, name: &str) -> Option<FnId> {
        let lower = name.to_ascii_lowercase();
        self.funcs.iter().position(|f| f.name == lower).map(|i| FnId(i as u32))
    }

    pub fn constant(&self, name: &str) -> Option<f32> {
        let lower = name.to_ascii_lowercase();
        self.consts.iter().find(|(n, _)| *n == lower).map(|(_, v)| *v)
    }

    pub fn var_count(&self) -> usize {
        self.vars.len()
    }

    pub fn fn_count(&self) -> usize {
        self.funcs.len()
    }

    pub fn fn_def(&self, id: FnId) -> Option<&FnDef> {
        self.funcs.get(id.index())
    }

    /// Lookup by the lowercase name the lexer produced
    pub(crate) fn lookup(&self, lower: &str) -> Lookup {
        if let Some(i) = self.vars.iter().position(|n| n == lower) {
            return Lookup::Var(VarId(i as u32));
        }
        if let Some((_, v)) = self.consts.iter().find(|(n, _)| n == lower) {
            return Lookup::Const(*v);
        }
        if let Some(i) = self.funcs.iter().position(|f| f.name == lower) {
            return Lookup::Fn(FnId(i as u32));
        }
        Lookup::None
    }
}

pub(crate) enum Lookup {
    Var(VarId),
    Const(f32),
    Fn(FnId),
    None,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_registration_order() {
        let mut s = Symbols::new();
        assert_eq!(s.add_var("t"), Ok(VarId(0)));
        assert_eq!(s.add_var("f"), Ok(VarId(1)));
        assert_eq!(s.add_fn("rand", 0, true), Ok(FnId(0)));
        assert_eq!(s.add_fn("integrate", 1, true), Ok(FnId(1)));
        assert_eq!(s.var("F"), Some(VarId(1)));
        assert_eq!(s.function("Integrate"), Some(FnId(1)));
    }

    #[test]
    fn names_are_checked() {
        let mut s = Symbols::new();
        assert_eq!(s.add_var(""), Err(SymbolError::Invalid));
        assert_eq!(s.add_var("1a"), Err(SymbolError::Invalid));
        assert_eq!(s.add_var("a-b"), Err(SymbolError::Invalid));
        assert_eq!(s.add_var("sin"), Err(SymbolError::Reserved));
        assert_eq!(s.add_var("and"), Err(SymbolError::Reserved));
        assert_eq!(s.add_var("pi"), Err(SymbolError::Duplicate));
        assert_eq!(s.add_var("A1"), Ok(VarId(0)));
        assert_eq!(s.add_var("a1"), Err(SymbolError::Duplicate));
        assert_eq!(s.add_fn("x", 9, false), Err(SymbolError::Invalid));
    }
}
