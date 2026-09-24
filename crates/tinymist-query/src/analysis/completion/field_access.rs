//! Completion for field access on nodes.

use typst::foundations::{Array, Styles};
use typst::syntax::ast::MathTextKind;

use crate::analysis::completion::typst_specific::ValueCompletionInfo;

use super::*;
impl CompletionPair<'_, '_, '_> {
    /// Add completions for all dot targets on a node.
    pub fn doc_access_completions(&mut self, target: &LinkedNode) -> Option<()> {
        self.static_dot_access_completions(target)
            .or_else(|| self.value_dot_access_completions(target))
            .or_else(|| self.type_dot_access_completions(target))
    }

    /// Complete known values and interfaces without tracing the document.
    ///
    /// Element functions still need tracing to recover the active styles. A
    /// content type alone also does not describe all fields of the actual value.
    /// Keep these, unknown types, and empty interfaces on the dynamic path.
    fn static_dot_access_completions(&mut self, target: &LinkedNode) -> Option<()> {
        let ty = self.worker.ctx.post_type_of_node(target.clone())?;
        if let Ty::Value(value) = &ty {
            if matches!(&value.val, Value::Type(_))
                || matches!(&value.val, Value::Func(func) if func.to_element().is_some())
            {
                return None;
            }
            return self.complete_dot_value(target, value.val.clone(), None);
        }

        // Array methods do not depend on the contents. Reuse the value renderer
        // so their receiver binding, snippets and details stay unchanged.
        if matches!(ty, Ty::Array(..) | Ty::Tuple(..)) {
            // An unevaluated function body uses the type fallback's signature
            // details. Keep the existing value/type choice inside closures.
            if node_ancestors(target).any(|node| node.kind() == SyntaxKind::Closure) {
                return None;
            }
            return self.complete_dot_value(target, Value::Array(Array::new()), None);
        }

        // Inferred record keys are not necessarily complete: mutation or a
        // computed key can add fields that only exist in the runtime value.
        if !matches!(ty, Ty::Builtin(BuiltinTy::Module(..)))
            || matches!(self.dot_access_mode(target), InterpretMode::Math)
        {
            return None;
        }

        let defines = self.type_field_access_definitions(&ty);
        if defines.defines.is_empty() {
            return None;
        }

        self.def_completions(defines, true);
        self.postfix_completions(target, ty);
        Some(())
    }

    /// Dot-access can sit inside a math equation while still targeting a code
    /// interpolation like `$ #calc. $`. In that case, the accessed expression's
    /// mode is the one that matters for completion behavior.
    fn dot_access_mode(&self, target: &LinkedNode) -> InterpretMode {
        let mode = self.cursor.leaf_mode();
        let target_mode = interpret_mode_at(Some(target));

        if matches!(mode, InterpretMode::Math) && matches!(target_mode, InterpretMode::Code) {
            return target_mode;
        }

        mode
    }

    /// Add completions for all fields on a type.
    fn type_dot_access_completions(&mut self, target: &LinkedNode) -> Option<()> {
        let mode = self.dot_access_mode(target);

        if matches!(mode, InterpretMode::Math) {
            return None;
        }

        self.type_field_access_completions(target)
    }

    /// Add completions for all fields on a type.
    fn type_field_access_completions(&mut self, target: &LinkedNode) -> Option<()> {
        let ty = self
            .worker
            .ctx
            .post_type_of_node(target.clone())
            .filter(|ty| !matches!(ty, Ty::Any));
        crate::log_debug_ct!("type_field_access_completions_on: {target:?} -> {ty:?}");
        let defines = self.type_field_access_definitions(&ty?);
        if defines.defines.is_empty() {
            return None;
        }
        self.def_completions(defines, true);
        Some(())
    }

    fn type_field_access_definitions(&mut self, ty: &Ty) -> Defines {
        let mut defines = Defines {
            types: self.worker.ctx.type_check(&self.cursor.source),
            defines: Default::default(),
            docs: Default::default(),
        };
        ty.iface_surface(
            true,
            &mut CompletionScopeChecker {
                check_kind: ScopeCheckKind::FieldAccess,
                defines: &mut defines,
                ctx: self.worker.ctx,
            },
        );

        defines
    }

    /// Add completions for all fields on a value.
    fn value_dot_access_completions(&mut self, target: &LinkedNode) -> Option<()> {
        let (value, styles) = self.worker.ctx.analyze_expr_first(target)?;

        self.complete_dot_value(target, value, styles.as_ref())
    }

    fn complete_dot_value(
        &mut self,
        target: &LinkedNode,
        value: Value,
        styles: Option<&Styles>,
    ) -> Option<()> {
        let mode = self.dot_access_mode(target);
        let valid_field_access_syntax =
            !matches!(mode, InterpretMode::Math) || is_valid_math_field_access(target);
        let valid_postfix_target =
            !matches!(mode, InterpretMode::Math) || is_valid_math_postfix(target);

        if !valid_field_access_syntax && !valid_postfix_target {
            return None;
        }

        if valid_field_access_syntax {
            self.value_field_access_completions(&value, mode);
        }
        if valid_postfix_target {
            self.postfix_completions(target, Ty::Value(InsTy::new(value.clone())));
        }

        match value {
            Value::Symbol(symbol) => {
                self.symbol_var_completions(&symbol, None);

                if valid_postfix_target {
                    self.ufcs_completions(target);
                }
            }
            Value::Content(content) => {
                if valid_field_access_syntax {
                    for (name, value) in content.fields() {
                        self.value_completion(Some(name.into()), &value, false, None);
                    }
                }
                if valid_postfix_target {
                    self.ufcs_completions(target);
                }
            }
            Value::Dict(dict) if valid_field_access_syntax => {
                for (name, value) in dict.iter() {
                    self.value_completion(Some(name.clone().into()), value, false, None);
                }
            }
            Value::Func(func) if valid_field_access_syntax => {
                // Autocomplete get rules.
                if let Some((elem, styles)) = func.to_element().zip(styles) {
                    for param in elem.params().iter().filter(|param| !param.required) {
                        if let Some(value) = elem
                            .field_id(param.name)
                            .map(|id| elem.field_from_styles(id, StyleChain::new(styles)))
                        {
                            self.value_completion(
                                Some(param.name.into()),
                                &value.unwrap(),
                                false,
                                None,
                            );
                        }
                    }
                }
            }
            _ => {}
        }

        Some(())
    }

    fn value_field_access_completions(&mut self, value: &Value, mode: InterpretMode) {
        let elem_parens = !matches!(mode, InterpretMode::Math);
        for (name, bind) in value.ty().scope().iter() {
            if matches!(mode, InterpretMode::Math) && is_func(bind.read()) {
                continue;
            }

            self.value_completion_(
                bind.read(),
                ValueCompletionInfo {
                    label: Some(name.clone()),
                    parens: elem_parens,
                    docs: None,
                    label_details: None,
                    bound_self: true,
                },
            );
        }

        if let Some(scope) = value.scope() {
            for (name, bind) in scope.iter() {
                if matches!(mode, InterpretMode::Math) && is_func(bind.read()) {
                    continue;
                }

                self.value_completion_(
                    bind.read(),
                    ValueCompletionInfo {
                        label: Some(name.clone()),
                        parens: elem_parens,
                        docs: None,
                        label_details: None,
                        bound_self: false,
                    },
                );
            }
        }

        for &field in fields_on(value.ty()) {
            // Complete the field name along with its value. Notes:
            // 1. No parentheses since function fields cannot currently be called
            // with method syntax;
            // 2. We can unwrap the field's value since it's a field belonging to
            // this value's type, so accessing it should not fail.
            self.value_completion_(
                &value.field(field, ()).unwrap(),
                ValueCompletionInfo {
                    label: Some(field.into()),
                    parens: false,
                    docs: None,
                    label_details: None,
                    bound_self: true,
                },
            );
        }
    }
}

fn is_func(read: &Value) -> bool {
    matches!(read, Value::Func(func) if func.to_element().is_none())
}

fn is_valid_math_field_access(target: &SyntaxNode) -> bool {
    if let Some(field_access) = target.cast::<ast::FieldAccess>() {
        return is_valid_math_field_access(field_access.target().to_untyped());
    }
    if let Some(field_access) = target.cast::<ast::MathFieldAccess>() {
        return is_valid_math_field_access(field_access.target().to_untyped());
    }
    if matches!(target.kind(), SyntaxKind::Ident | SyntaxKind::MathIdent) {
        return true;
    }

    false
}

fn is_valid_math_postfix(target: &SyntaxNode) -> bool {
    fn bad_punc_text(punc: char) -> bool {
        punc.is_ascii_punctuation() || punc.is_ascii_whitespace()
    }

    if let Some(target) = target.cast::<ast::MathText>() {
        return match target.get() {
            MathTextKind::Grapheme(ch) => !ch.chars().any(bad_punc_text),
            MathTextKind::Number(..) => true,
        };
    }

    if let Some(target) = target.cast::<ast::Text>() {
        let target = target.get();
        return !target.is_empty() && target.chars().all(|ch| !bad_punc_text(ch));
    }

    true
}

#[cfg(test)]
mod tests {
    use tinymist_analysis::stats::GLOBAL_STATS;

    use super::*;
    use crate::{
        CompletionRequest,
        tests::{run_with_ctx, run_with_sources},
    };

    fn trace_count(source: &Source) -> u64 {
        let file = format!("{:?}", source.id()).replace('\\', "/");
        GLOBAL_STATS
            .report_json()
            .into_iter()
            .filter(|entry| entry.file.as_deref() == Some(file.as_str()))
            .filter(|entry| entry.query == "analyze_expr")
            .map(|entry| entry.count)
            .sum()
    }

    fn check_completion(
        text: &str,
        target: &str,
        includes: &[&str],
        excludes: &[&str],
        traced: bool,
    ) -> Vec<CompletionItem> {
        run_with_sources(text, |verse, path| {
            run_with_ctx(verse, path, &|ctx, path| {
                assert!(!typst_shim::is_syntax_only());
                let source = ctx.source_by_path(&path).unwrap();
                let cursor = source.text().rfind(target).unwrap() + target.len();
                let before = trace_count(&source);
                let result = CompletionRequest {
                    path,
                    position: ctx.to_lsp_pos(cursor, &source),
                    explicit: false,
                    trigger_character: Some('.'),
                }
                .request(ctx)
                .unwrap();
                let labels = result
                    .items
                    .iter()
                    .map(|item| item.label.as_str())
                    .collect::<Vec<_>>();
                for name in includes {
                    assert!(labels.contains(name), "missing {name:?}: {labels:?}");
                }
                for name in excludes {
                    assert!(!labels.contains(name), "unexpected {name:?}: {labels:?}");
                }
                let after = trace_count(&source);
                if traced {
                    assert!(
                        after > before,
                        "runtime-only completion must retain tracing"
                    );
                } else {
                    assert_eq!(after, before, "static completion must not trace");
                }

                // The evaluation shortcut must select exactly the value and
                // styles that the old full-document trace selected, including
                // dictionary mutations and values evaluated during layout.
                let root = LinkedNode::new(source.root());
                let node = root.leaf_at_compat(cursor - 1).unwrap();
                let first = ctx.analyze_expr_first(&node);
                let full = ctx.analyze_expr(&node).into_iter().next();
                assert!(first.is_some());
                assert_eq!(first, full);
                if traced {
                    assert!(first.unwrap().1.is_some(), "contextual styles were lost");
                }
                result.items
            })
        })
    }

    #[test]
    fn static_dot_completion_keeps_module_exports_without_tracing() {
        let items = check_completion(
            r#"
/// path: completion-static-exports.typ
#let member(x) = 1
-----
/// path: completion-static-module.typ
#import "completion-static-exports.typ": member
#let alias = calc
#alias.abs(1)
#import "completion-static-exports.typ" as local
#local.member(1)
"#,
            "#local.",
            &["member", "if", "let"],
            &[],
            false,
        );
        let member = items.iter().find(|item| item.label == "member").unwrap();
        assert_eq!(member.detail.as_deref(), Some("(any) => 1"));
        assert_eq!(
            member
                .label_details
                .as_ref()
                .and_then(|details| details.description.as_deref()),
            Some("(any) => 1")
        );
        let conditional = items.iter().find(|item| item.label == "if").unwrap();
        assert_eq!(conditional.kind, CompletionKind::Syntax);
        assert_eq!(conditional.detail.as_deref(), Some("wrap as if expression"));
    }

    #[test]
    fn static_dot_completion_keeps_builtin_alias_without_tracing() {
        check_completion(
            r#"
/// path: completion-static-builtin-alias.typ
#let alias = calc
#alias.abs(1)
"#,
            "#alias.",
            &["abs", "odd"],
            &[],
            false,
        );
    }

    #[test]
    fn static_dot_completion_keeps_math_symbols_and_postfix_without_tracing() {
        check_completion(
            r#"
/// path: completion-static-symbol.typ
$ arrow.b $
"#,
            "arrow.",
            &["b", "abs"],
            &[],
            false,
        );
    }

    #[test]
    fn static_dot_completion_keeps_code_interpolation_in_math_without_tracing() {
        check_completion(
            r#"
/// path: completion-static-math-module.typ
$ #calc.abs(1) $
"#,
            "#calc.",
            &["abs", "odd"],
            &[],
            false,
        );
    }

    #[test]
    fn static_dot_completion_keeps_array_methods_without_tracing() {
        check_completion(
            r#"
/// path: completion-static-array.typ
#let entries = (1, 2)
#entries.len()
"#,
            "#entries.",
            &["len", "map"],
            &[],
            false,
        );
    }

    #[test]
    fn dot_completion_keeps_called_closure_array_method_details() {
        let items = check_completion(
            r#"
/// path: completion-called-closure-array.typ
#let flatten(entries) = entries.flatten()
#flatten(((1,),))
"#,
            "entries.",
            &["flatten", "map"],
            &[],
            false,
        );
        let flatten = items.iter().find(|item| item.label == "flatten").unwrap();
        assert!(flatten.label_details.is_none());
        assert_eq!(
            flatten.text_edit.as_ref().unwrap().new_text().as_str(),
            "flatten()${1:}"
        );
    }

    #[test]
    fn dot_completion_evaluates_runtime_dictionary_keys_without_layout() {
        check_completion(
            r#"
/// path: completion-dynamic-dictionary.typ
#let make() = {
  let result = (known: 0)
  for key in ("runtime",) { result.insert(key, 1) }
  result
}
#let object = make()
#object.runtime
"#,
            "#object.",
            &["known", "runtime", "keys"],
            &[],
            false,
        );
    }

    #[test]
    fn dot_completion_traces_active_element_styles() {
        check_completion(
            r#"
/// path: completion-dynamic-styles.typ
#set text(fill: red)
#context text.fill
"#,
            "text.",
            &["fill", "where"],
            &[],
            true,
        );
    }

    #[test]
    fn dot_completion_keeps_contextual_callback_dictionary() {
        check_completion(
            r#"
/// path: completion-contextual-callback.typ
#let scope(body) = context {
  let value = (known: 0)
  for key in ("runtime",) { value.insert(key, 1) }
  body(value)
}
#scope(s => [#s.runtime])
"#,
            "#s.",
            &["known", "runtime", "keys"],
            &[],
            true,
        );
    }

    #[test]
    fn dot_completion_keeps_first_value_before_contextual_observations() {
        check_completion(
            r#"
/// path: completion-first-value-order.typ
#let describe(value) = value.keys()
#describe((first: 1))
#context describe((later: 2))
"#,
            "value.",
            &["first", "keys"],
            &["later"],
            false,
        );
    }

    #[test]
    fn dot_completion_preserves_builtin_shadowing() {
        check_completion(
            r#"
/// path: completion-shadowed-builtin.typ
#let calc = (mine: 0)
#calc.mine
"#,
            "#calc.",
            &["mine", "keys"],
            &["odd"],
            false,
        );
    }
}
