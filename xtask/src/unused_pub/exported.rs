//! What a library crate exports: each item marked `pub` in the modules its root declares,
//! read with `syn`, tests aside, and the names each one's interface holds. `unreachable_pub`
//! already makes every such item reachable from the crate's root.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use syn::visit::{self, Visit};
use syn::{Attribute, Fields, ImplItem, Item, ItemImpl, Type, UseTree, Visibility};

/// An item a library crate exports: its name, what kind of item it is, and its file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Exported {
    pub(crate) name: String,
    pub(crate) kind: &'static str,
    pub(crate) file: String,
}

/// A library crate: its files, what they export, and the names each exported name's interface
/// holds (its signature, its public fields' types, its variants, what a trait asks), which
/// must be public while it is.
#[derive(Debug, Default)]
pub(crate) struct Library {
    /// The files of the crate's module tree, by path from the workspace root.
    pub(crate) files: BTreeSet<String>,
    pub(crate) exported: Vec<Exported>,
    pub(crate) interfaces: BTreeMap<String, BTreeSet<String>>,
}

/// Reads the library whose root is `lib`, a path from `root`, the workspace root, following
/// each `mod` its files declare; `foo.rs` declares its modules in `foo/`.
pub(crate) fn read(root: &Path, lib: &str) -> Result<Library, String> {
    let children = lib
        .rsplit_once('/')
        .map_or_else(String::new, |(dir, _)| format!("{dir}/"));
    let mut library = Library::default();
    library.file(root, lib, &children)?;
    Ok(library)
}

/// Reads one source's items: what it exports, and the names in their interfaces.
#[cfg(test)]
pub(crate) fn read_text(text: &str) -> Result<Library, String> {
    let items = syn::parse_file(text).map_err(|err| err.to_string())?.items;
    let mut library = Library::default();
    library.items(Path::new(""), &items, "lib.rs", "")?;
    Ok(library)
}

impl Library {
    /// Reads the module in `path`, whose own modules are in `children`.
    fn file(&mut self, root: &Path, path: &str, children: &str) -> Result<(), String> {
        let text = std::fs::read_to_string(root.join(path))
            .map_err(|err| format!("could not read {path}: {err}"))?;
        let parsed =
            syn::parse_file(&text).map_err(|err| format!("could not parse {path}: {err}"))?;
        self.files.insert(path.to_owned());
        self.items(root, &parsed.items, path, children)
    }

    fn items(
        &mut self,
        root: &Path,
        items: &[Item],
        path: &str,
        children: &str,
    ) -> Result<(), String> {
        for item in items.iter().filter(|item| !tested(item)) {
            match item {
                Item::Mod(module) => {
                    self.public(&module.vis, &module.ident, "mod", path, BTreeSet::new());
                    let dir = format!("{children}{}/", module.ident);
                    match &module.content {
                        Some((_, inner)) => self.items(root, inner, path, &dir)?,
                        None => self.file(root, &format!("{children}{}.rs", module.ident), &dir)?,
                    }
                }
                Item::Impl(block) => self.implemented(block, path),
                Item::Use(used) if matches!(used.vis, Visibility::Public(_)) => {
                    self.reexported(&used.tree, None, path);
                }
                Item::Macro(defined) if macro_exported(&defined.attrs) => {
                    if let Some(name) = &defined.ident {
                        let body = defined.mac.tokens.to_string();
                        let interface = super::words(&body).map(str::to_owned);
                        self.add(name, "macro", path, interface.collect());
                    }
                }
                item => self.named(item, path),
            }
        }
        Ok(())
    }

    /// An impl block's public methods and constants; a trait's, the types it names for the
    /// trait, which are part of the interface of the type it is for.
    fn implemented(&mut self, block: &ItemImpl, path: &str) {
        if block.trait_.is_some() {
            let Type::Path(ty) = &*block.self_ty else {
                return;
            };
            let Some(last) = ty.path.segments.last() else {
                return;
            };
            let named = names(|names| {
                for item in &block.items {
                    if let ImplItem::Type(assigned) = item {
                        names.visit_type(&assigned.ty);
                    }
                }
            });
            self.interfaces
                .entry(unraw(&last.ident))
                .or_default()
                .extend(named);
            return;
        }
        let generics = names(|names| names.visit_generics(&block.generics));
        for item in &block.items {
            match item {
                ImplItem::Fn(method) if !for_tests(&method.attrs) => {
                    let mut interface = names(|names| names.visit_signature(&method.sig));
                    interface.extend(generics.iter().cloned());
                    self.public(&method.vis, &method.sig.ident, "fn", path, interface);
                }
                ImplItem::Const(constant) if !for_tests(&constant.attrs) => {
                    let interface = names(|names| names.visit_type(&constant.ty));
                    self.public(&constant.vis, &constant.ident, "const", path, interface);
                }
                _ => {}
            }
        }
    }

    /// An item with a name and a visibility of its own, and a struct's public fields.
    fn named(&mut self, item: &Item, path: &str) {
        let (vis, ident, kind, interface) = match item {
            Item::Fn(f) => (
                &f.vis,
                &f.sig.ident,
                "fn",
                names(|n| n.visit_signature(&f.sig)),
            ),
            Item::Struct(s) => (&s.vis, &s.ident, "struct", public_types(&s.fields)),
            Item::Union(u) => (&u.vis, &u.ident, "union", names(|n| n.visit_item_union(u))),
            Item::Enum(e) => (&e.vis, &e.ident, "enum", names(|n| n.visit_item_enum(e))),
            Item::Trait(t) => (&t.vis, &t.ident, "trait", names(|n| n.visit_item_trait(t))),
            Item::TraitAlias(t) => {
                let interface = names(|n| n.visit_item_trait_alias(t));
                (&t.vis, &t.ident, "trait", interface)
            }
            Item::Type(t) => (&t.vis, &t.ident, "type", names(|n| n.visit_item_type(t))),
            Item::Const(c) => (&c.vis, &c.ident, "const", names(|n| n.visit_type(&c.ty))),
            Item::Static(s) => (&s.vis, &s.ident, "static", names(|n| n.visit_type(&s.ty))),
            _ => return,
        };
        self.public(vis, ident, kind, path, interface);
        // A crate's own struct may mark its fields `pub`; only an exported struct exports them.
        if let Item::Struct(s) = item
            && matches!(s.vis, Visibility::Public(_))
        {
            for field in &s.fields {
                if let Some(name) = &field.ident {
                    let interface = names(|n| n.visit_type(&field.ty));
                    self.public(&field.vis, name, "field", path, interface);
                }
            }
        }
    }

    /// The names a `pub use` gives its crate's interface: each last segment, or its new name,
    /// whose interface is the name it renames; a glob names none. `parent` is the segment
    /// before, which `self` stands for.
    fn reexported(&mut self, tree: &UseTree, parent: Option<&syn::Ident>, path: &str) {
        match tree {
            UseTree::Path(prefix) => self.reexported(&prefix.tree, Some(&prefix.ident), path),
            UseTree::Name(used) if used.ident == "self" => {
                if let Some(parent) = parent {
                    self.add(parent, "use", path, BTreeSet::new());
                }
            }
            UseTree::Name(used) => self.add(&used.ident, "use", path, BTreeSet::new()),
            UseTree::Rename(renamed) => {
                let original = BTreeSet::from([unraw(&renamed.ident)]);
                self.add(&renamed.rename, "use", path, original);
            }
            UseTree::Group(group) => {
                for item in &group.items {
                    self.reexported(item, parent, path);
                }
            }
            UseTree::Glob(_) => {}
        }
    }

    fn public(
        &mut self,
        vis: &Visibility,
        ident: &syn::Ident,
        kind: &'static str,
        path: &str,
        interface: BTreeSet<String>,
    ) {
        if matches!(vis, Visibility::Public(_)) {
            self.add(ident, kind, path, interface);
        }
    }

    fn add(
        &mut self,
        ident: &syn::Ident,
        kind: &'static str,
        path: &str,
        interface: BTreeSet<String>,
    ) {
        let name = unraw(ident);
        if name.starts_with('_') {
            return;
        }
        self.interfaces
            .entry(name.clone())
            .or_default()
            .extend(interface);
        self.exported.push(Exported {
            name,
            kind,
            file: path.to_owned(),
        });
    }
}

/// The names in the types of the public ones of `fields`.
fn public_types(fields: &Fields) -> BTreeSet<String> {
    names(|names| {
        for field in fields {
            if matches!(field.vis, Visibility::Public(_)) {
                names.visit_type(&field.ty);
            }
        }
    })
}

/// The names a part of the syntax holds.
#[derive(Default)]
struct Names(BTreeSet<String>);

impl<'ast> Visit<'ast> for Names {
    fn visit_ident(&mut self, ident: &'ast syn::Ident) {
        self.0.insert(unraw(ident));
        visit::visit_ident(self, ident);
    }
}

fn names(visit: impl FnOnce(&mut Names)) -> BTreeSet<String> {
    let mut names = Names::default();
    visit(&mut names);
    names.0
}

/// An identifier as its callers write it, `r#` aside.
fn unraw(ident: &syn::Ident) -> String {
    let name = ident.to_string();
    name.strip_prefix("r#").map(str::to_owned).unwrap_or(name)
}

/// Whether `item` is compiled only for its crate's own tests.
fn tested(item: &Item) -> bool {
    let attrs = match item {
        Item::Const(i) => &i.attrs,
        Item::Enum(i) => &i.attrs,
        Item::Fn(i) => &i.attrs,
        Item::Impl(i) => &i.attrs,
        Item::Macro(i) => &i.attrs,
        Item::Mod(i) => &i.attrs,
        Item::Static(i) => &i.attrs,
        Item::Struct(i) => &i.attrs,
        Item::Trait(i) => &i.attrs,
        Item::TraitAlias(i) => &i.attrs,
        Item::Type(i) => &i.attrs,
        Item::Union(i) => &i.attrs,
        Item::Use(i) => &i.attrs,
        _ => return false,
    };
    for_tests(attrs)
}

/// Whether `attrs` hold `#[cfg(test)]`.
fn for_tests(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr
                .parse_args::<syn::Ident>()
                .is_ok_and(|arg| arg == "test")
    })
}

/// Whether `attrs` hold `#[macro_export]`.
fn macro_exported(attrs: &[Attribute]) -> bool {
    attrs
        .iter()
        .any(|attr| attr.path().is_ident("macro_export"))
}
