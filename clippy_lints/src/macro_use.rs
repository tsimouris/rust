use crate::utils::{in_macro, snippet, span_lint_and_sugg};
use hir::def::{DefKind, Res};
use if_chain::if_chain;
use rustc_ast::ast;
use rustc_data_structures::fx::{FxHashMap, FxHashSet};
use rustc_errors::Applicability;
use rustc_hir as hir;
use rustc_lint::{LateContext, LateLintPass, LintContext};
use rustc_session::{declare_tool_lint, impl_lint_pass};
use rustc_span::{edition::Edition, Span};

declare_clippy_lint! {
    /// **What it does:** Checks for `#[macro_use] use...`.
    ///
    /// **Why is this bad?** Since the Rust 2018 edition you can import
    /// macro's directly, this is considered idiomatic.
    ///
    /// **Known problems:** None.
    ///
    /// **Example:**
    /// ```rust
    /// #[macro_use]
    /// use lazy_static;
    /// ```
    pub MACRO_USE_IMPORTS,
    pedantic,
    "#[macro_use] is no longer needed"
}

const BRACKETS: &[char] = &['<', '>'];

#[derive(Clone, Debug, PartialEq, Eq)]
struct PathAndSpan {
    path: String,
    span: Span,
}

/// `MacroRefData` includes the name of the macro
/// and the path from `SourceMap::span_to_filename`.
#[derive(Debug, Clone)]
pub struct MacroRefData {
    name: String,
    path: String,
}

impl MacroRefData {
    pub fn new(name: String, callee: Span, cx: &LateContext<'_, '_>) -> Self {
        let mut path = cx.sess().source_map().span_to_filename(callee).to_string();

        // std lib paths are <::std::module::file type>
        // so remove brackets, space and type.
        if path.contains('<') {
            path = path.replace(BRACKETS, "");
        }
        if path.contains(' ') {
            path = path.split(' ').next().unwrap().to_string();
        }
        Self { name, path }
    }
}

#[derive(Default)]
#[allow(clippy::module_name_repetitions)]
pub struct MacroUseImports {
    /// the actual import path used and the span of the attribute above it.
    imports: Vec<(String, Span)>,
    /// the span of the macro reference, kept to ensure only one reference is used per macro call.
    collected: FxHashSet<Span>,
    mac_refs: Vec<MacroRefData>,
}

impl_lint_pass!(MacroUseImports => [MACRO_USE_IMPORTS]);

impl MacroUseImports {
    fn push_unique_macro(&mut self, cx: &LateContext<'_, '_>, span: Span) {
        let call_site = span.source_callsite();
        let name = snippet(cx, cx.sess().source_map().span_until_char(call_site, '!'), "_");
        if let Some(callee) = span.source_callee() {
            if !self.collected.contains(&call_site) {
                let name = if name.contains("::") {
                    name.split("::").last().unwrap().to_string()
                } else {
                    name.to_string()
                };

                self.mac_refs.push(MacroRefData::new(name, callee.def_site, cx));
                self.collected.insert(call_site);
            }
        }
    }

    fn push_unique_macro_pat_ty(&mut self, cx: &LateContext<'_, '_>, span: Span) {
        let call_site = span.source_callsite();
        let name = snippet(cx, cx.sess().source_map().span_until_char(call_site, '!'), "_");
        if let Some(callee) = span.source_callee() {
            if !self.collected.contains(&call_site) {
                self.mac_refs
                    .push(MacroRefData::new(name.to_string(), callee.def_site, cx));
                self.collected.insert(call_site);
            }
        }
    }
}

impl<'l, 'txc> LateLintPass<'l, 'txc> for MacroUseImports {
    fn check_item(&mut self, cx: &LateContext<'_, '_>, item: &hir::Item<'_>) {
        if_chain! {
            if cx.sess().opts.edition == Edition::Edition2018;
            if let hir::ItemKind::Use(path, _kind) = &item.kind;
            if let Some(mac_attr) = item
                .attrs
                .iter()
                .find(|attr| attr.ident().map(|s| s.to_string()) == Some("macro_use".to_string()));
            if let Res::Def(DefKind::Mod, id) = path.res;
            then {
                for kid in cx.tcx.item_children(id).iter() {
                    if let Res::Def(DefKind::Macro(_mac_type), mac_id) = kid.res {
                        let span = mac_attr.span;
                        let def_path = cx.tcx.def_path_str(mac_id);
                        self.imports.push((def_path, span));
                    }
                }
            } else {
                if in_macro(item.span) {
                    self.push_unique_macro_pat_ty(cx, item.span);
                }
            }
        }
    }
    fn check_attribute(&mut self, cx: &LateContext<'_, '_>, attr: &ast::Attribute) {
        if in_macro(attr.span) {
            self.push_unique_macro(cx, attr.span);
        }
    }
    fn check_expr(&mut self, cx: &LateContext<'_, '_>, expr: &hir::Expr<'_>) {
        if in_macro(expr.span) {
            self.push_unique_macro(cx, expr.span);
        }
    }
    fn check_stmt(&mut self, cx: &LateContext<'_, '_>, stmt: &hir::Stmt<'_>) {
        if in_macro(stmt.span) {
            self.push_unique_macro(cx, stmt.span);
        }
    }
    fn check_pat(&mut self, cx: &LateContext<'_, '_>, pat: &hir::Pat<'_>) {
        if in_macro(pat.span) {
            self.push_unique_macro_pat_ty(cx, pat.span);
        }
    }
    fn check_ty(&mut self, cx: &LateContext<'_, '_>, ty: &hir::Ty<'_>) {
        if in_macro(ty.span) {
            self.push_unique_macro_pat_ty(cx, ty.span);
        }
    }
    #[allow(clippy::too_many_lines)]
    fn check_crate_post(&mut self, cx: &LateContext<'_, '_>, _krate: &hir::Crate<'_>) {
        let mut used = FxHashMap::default();
        let mut check_dup = vec![];
        for (import, span) in &self.imports {
            let found_idx = self.mac_refs.iter().position(|mac| import.ends_with(&mac.name));

            if let Some(idx) = found_idx {
                let _ = self.mac_refs.remove(idx);
                let seg = import.split("::").collect::<Vec<_>>();

                match seg.as_slice() {
                    [] => unreachable!("this should never be empty"),
                    [_] => unreachable!("path must have two segments ?"),
                    [root, item] => {
                        if !check_dup.contains(&item.to_string()) {
                            used.entry((root.to_string(), span))
                                .or_insert(vec![])
                                .push(item.to_string());
                            check_dup.push(item.to_string());
                        }
                    },
                    [root, rest @ ..] => {
                        if !rest.iter().all(|item| !check_dup.contains(&item.to_string())) {
                            let mut rest = rest.to_vec();
                            rest.sort();
                            used.entry((root.to_string(), span))
                                .or_insert(vec![])
                                .push(rest.join("::"));
                            check_dup.extend(rest.iter().map(ToString::to_string));
                        } else {
                            let mut filtered = rest
                                .iter()
                                .filter(|item| !check_dup.contains(&item.to_string()))
                                .map(ToString::to_string)
                                .collect::<Vec<_>>();
                            filtered.sort();
                            used.entry((root.to_string(), span))
                                .or_insert(vec![])
                                .push(filtered.join("::"));
                            check_dup.extend(filtered);
                        }
                    },
                }
            }
        }

        let mut suggestions = vec![];
        for ((root, span), path) in used {
            if path.len() == 1 {
                suggestions.push((span, format!("{}::{}", root, path[0])))
            } else {
                suggestions.push((span, format!("{}::{{{}}}", root, path.join(", "))))
            }
        }

        // If mac_refs is not empty we have encountered an import we could not handle
        // such as `std::prelude::v1::foo` or some other macro that expands to an import.
        if self.mac_refs.is_empty() {
            for (span, import) in suggestions {
                let help = format!("use {}", import);
                span_lint_and_sugg(
                    cx,
                    MACRO_USE_IMPORTS,
                    *span,
                    "`macro_use` attributes are no longer needed in the Rust 2018 edition",
                    "remove the attribute and import the macro directly, try",
                    help,
                    Applicability::MaybeIncorrect,
                )
            }
        }
    }
}
