//! Static lookup resolution for module tables and instances. The resolver is
//! deliberately conservative: any dynamic link leaves the lookup Unknown.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::checks::bindings::Bindings;
use crate::explain::LookupResult;
use crate::exports::{ExportShape, Member};

#[derive(Debug, Clone)]
pub(crate) enum LookupTarget {
    Module(PathBuf),
    Instance(PathBuf),
}

#[derive(Debug, Clone)]
pub(crate) struct LookupResolution {
    pub steps: Vec<String>,
    pub result: LookupResult,
    pub member: Option<Member>,
}

pub(crate) struct LookupContext<'a> {
    shapes: &'a HashMap<PathBuf, ExportShape>,
    bindings: &'a HashMap<PathBuf, Bindings>,
}

impl<'a> LookupContext<'a> {
    pub(crate) fn new(
        shapes: &'a HashMap<PathBuf, ExportShape>,
        bindings: &'a HashMap<PathBuf, Bindings>,
    ) -> Self {
        Self { shapes, bindings }
    }

    pub(crate) fn resolve(&self, target: LookupTarget, member: &str) -> LookupResolution {
        let mut walk = Walk::default();
        let found = match target {
            LookupTarget::Module(path) => self.module_lookup(&path, member, &mut walk),
            LookupTarget::Instance(path) => self.instance_lookup(&path, member, &mut walk),
        };
        let result = if let Some((path, kind)) = found {
            LookupResult::Found(format!("`{member}` resolved in {}", path.display()))
                .with_member(kind, &mut walk)
        } else if let Some(reason) = walk.unknown {
            LookupResult::Unknown(reason)
        } else {
            LookupResult::NotFound
        };
        LookupResolution {
            steps: walk.steps,
            result,
            member: walk.member,
        }
    }

    fn module_lookup(
        &self,
        path: &PathBuf,
        member: &str,
        walk: &mut Walk,
    ) -> Option<(PathBuf, Member)> {
        let key = ("module".to_string(), path.clone());
        if !walk.visited.insert(key.clone()) {
            walk.mark_unknown(format!("cycle while resolving {}", path.display()));
            return None;
        }
        let result = self.module_lookup_inner(path, member, walk);
        walk.visited.remove(&key);
        result
    }

    fn module_lookup_inner(
        &self,
        path: &PathBuf,
        member: &str,
        walk: &mut Walk,
    ) -> Option<(PathBuf, Member)> {
        walk.steps
            .push(format!("raw table lookup: {}.{member}", path.display()));
        let Some(shape) = self.shapes.get(path) else {
            walk.mark_unknown(format!(
                "module {} has an unknown export shape",
                path.display()
            ));
            return None;
        };
        if let Some(kind) = shape.members.get(member) {
            walk.steps
                .push(format!("found direct member on {}", path.display()));
            return Some((path.clone(), kind.clone()));
        }
        let Some(class) = &shape.class else {
            return None;
        };
        if let Some(meta) = &class.metatable_name {
            walk.steps.push(format!("metatable __index: {meta}"));
            if let Some(target) = self.binding(path, meta) {
                return self.module_lookup(target, member, walk);
            }
            walk.mark_unknown(format!(
                "metatable target `{meta}` is not a known module binding"
            ));
        } else if class.metatable_unknown {
            walk.mark_unknown(format!(
                "{} has a dynamic metatable __index",
                path.display()
            ));
        }
        None
    }

    fn instance_lookup(
        &self,
        path: &PathBuf,
        member: &str,
        walk: &mut Walk,
    ) -> Option<(PathBuf, Member)> {
        let key = ("instance".to_string(), path.clone());
        if !walk.visited.insert(key.clone()) {
            walk.mark_unknown(format!("cycle while resolving {}", path.display()));
            return None;
        }
        let result = self.instance_lookup_inner(path, member, walk);
        walk.visited.remove(&key);
        result
    }

    fn instance_lookup_inner(
        &self,
        path: &PathBuf,
        member: &str,
        walk: &mut Walk,
    ) -> Option<(PathBuf, Member)> {
        walk.steps
            .push(format!("raw instance lookup: {}.{member}", path.display()));
        let Some(shape) = self.shapes.get(path) else {
            walk.mark_unknown(format!(
                "class {} has an unknown export shape",
                path.display()
            ));
            return None;
        };
        let Some(class) = &shape.class else {
            walk.mark_unknown(format!("{} is not a recognized class", path.display()));
            return None;
        };
        if class.instance_members.contains(member) {
            walk.steps
                .push(format!("found instance member on {}", path.display()));
            return Some((path.clone(), Member::Value));
        }
        if let Some(parent) = &class.parent_constructor {
            walk.steps
                .push(format!("constructor parent: {parent}.new()"));
            if let Some(target) = self.binding(path, parent) {
                if let Some(found) = self.instance_lookup(target, member, walk) {
                    return Some(found);
                }
            } else {
                walk.mark_unknown(format!(
                    "parent constructor `{parent}` is not a known module binding"
                ));
            }
        }
        if class.index_self {
            walk.steps.push(format!("__index: {}", path.display()));
            if let Some(found) = self.module_lookup(path, member, walk) {
                return Some(found);
            }
        } else if let Some(other) = &class.index_other {
            walk.steps.push(format!("__index: {other}"));
            if let Some(target) = self.binding(path, other) {
                if let Some(found) = self.module_lookup(target, member, walk) {
                    return Some(found);
                }
            } else {
                walk.mark_unknown(format!(
                    "__index target `{other}` is not a known module binding"
                ));
            }
        } else if class.index_unknown {
            walk.mark_unknown(format!("{} has a dynamic __index", path.display()));
        }
        None
    }

    fn binding(&self, owner: &PathBuf, name: &str) -> Option<&PathBuf> {
        self.bindings.get(owner)?.get_module(name)
    }
}

#[derive(Default)]
struct Walk {
    steps: Vec<String>,
    visited: HashSet<(String, PathBuf)>,
    unknown: Option<String>,
    member: Option<Member>,
}

impl Walk {
    fn mark_unknown(&mut self, reason: String) {
        self.unknown.get_or_insert(reason);
    }
}

trait FoundMember {
    fn with_member(self, member: Member, walk: &mut Walk) -> Self;
}

impl FoundMember for LookupResult {
    fn with_member(self, member: Member, walk: &mut Walk) -> Self {
        walk.member = Some(member);
        self
    }
}
