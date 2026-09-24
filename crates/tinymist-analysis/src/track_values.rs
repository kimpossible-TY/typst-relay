//! Dynamic analysis of an expression or import statement.

use comemo::Track;
use ecow::*;
use tinymist_std::typst::{TypstDocument, TypstPagedDocument};
use typst::World;
use typst::engine::{Engine, Route, Sink, Traced};
use typst::foundations::{Context, Label, Scopes, Styles, Value};
use typst::introspection::EmptyIntrospector;
use typst::model::BibliographyElem;
use typst::syntax::{LinkedNode, Span, SyntaxKind, SyntaxNode, ast};
use typst_shim::eval::Vm;
use typst_shim::is_syntax_only;

use crate::stats::GLOBAL_STATS;

/// Try to determine a set of possible values for an expression.
pub fn analyze_expr(world: &dyn World, node: &LinkedNode) -> EcoVec<(Value, Option<Styles>)> {
    if let Some(parent) = node.parent()
        && parent.kind() == SyntaxKind::FieldAccess
        && node.index() > 0
    {
        return analyze_expr(world, parent);
    }

    analyze_expr_(world, node.get())
}

/// Try to determine a set of possible values for an expression.
#[typst_macros::time(span = node.span())]
pub fn analyze_expr_(world: &dyn World, node: &SyntaxNode) -> EcoVec<(Value, Option<Styles>)> {
    let Some(expr) = node.cast::<ast::Expr>() else {
        return eco_vec![];
    };

    let val = match expr {
        ast::Expr::None(_) => Value::None,
        ast::Expr::Auto(_) => Value::Auto,
        ast::Expr::Bool(v) => Value::Bool(v.get()),
        ast::Expr::Int(v) => Value::Int(v.get()),
        ast::Expr::Float(v) => Value::Float(v.get()),
        ast::Expr::Numeric(v) => Value::numeric(v.get()),
        ast::Expr::Str(v) => Value::Str(v.get().into()),
        _ => {
            if node.kind() == SyntaxKind::Contextual
                && let Some(child) = node.children().last()
            {
                return analyze_expr_(world, child);
            }

            // Only traces if not in syntax-only mode because typst::trace requires
            // compilation information.
            if is_syntax_only() {
                return eco_vec![];
            }

            let _guard = GLOBAL_STATS.stat(node.span().id(), "analyze_expr");
            return typst::trace::<TypstPagedDocument>(world, node.span());
        }
    };

    eco_vec![(val, None)]
}

/// Determines the first value observed for an expression.
///
/// Evaluation observations precede layout observations in [`typst::trace`]. If
/// evaluation already observes a value, consumers that only use the first one
/// can skip layout entirely without losing runtime fields or changing which
/// value they select. Contextual expressions still require the full trace.
pub fn analyze_expr_first_(
    world: &dyn World,
    node: &SyntaxNode,
) -> Option<(Value, Option<Styles>)> {
    let expr = node.cast::<ast::Expr>()?;
    if matches!(
        expr,
        ast::Expr::None(_)
            | ast::Expr::Auto(_)
            | ast::Expr::Bool(_)
            | ast::Expr::Int(_)
            | ast::Expr::Float(_)
            | ast::Expr::Numeric(_)
            | ast::Expr::Str(_)
    ) || is_syntax_only()
    {
        return analyze_expr_(world, node).into_iter().next();
    }

    if node.kind() == SyntaxKind::Contextual
        && let Some(child) = node.children().last()
    {
        return analyze_expr_first_(world, child);
    }

    let first = {
        let _guard = GLOBAL_STATS.stat(node.span().id(), "analyze_expr_eval");
        let main = world.source(world.main()).ok()?;
        let traced = Traced::new(node.span());
        let mut sink = Sink::new();
        // Retain observations even when later evaluation fails, matching trace.
        let _ = typst_shim::eval::eval(
            world.track(),
            world.library(),
            traced.track(),
            sink.track_mut(),
            Route::default().track(),
            &main,
        );
        sink.values().into_iter().next()
    };

    first.or_else(|| analyze_expr_(world, node).into_iter().next())
}

/// Try to load a module from the current source file.
#[typst_macros::time(span = source.span())]
pub fn analyze_import_(world: &dyn World, source: &SyntaxNode) -> (Option<Value>, Option<Value>) {
    let source_span = source.span();
    let Some((source, _)) = analyze_expr_(world, source).into_iter().next() else {
        return (None, None);
    };
    if source.scope().is_some() {
        return (Some(source.clone()), Some(source));
    }

    let _guard = GLOBAL_STATS.stat(source_span.id(), "analyze_import");

    let library = world.library();
    let introspector = EmptyIntrospector;
    let traced = Traced::default();
    let mut sink = Sink::new();
    let engine = Engine {
        library,
        world: world.track(),
        route: Route::default(),
        introspector: typst::utils::Protected::new(introspector.track()),
        traced: traced.track(),
        sink: sink.track_mut(),
    };

    let context = Context::none();
    let mut vm = Vm::new(
        engine,
        context.track(),
        Scopes::new(Some(library)),
        Span::detached(),
    );
    let module = match source.clone() {
        Value::Str(path) => typst_shim::eval::import(&mut vm.engine, &path, source_span)
            .ok()
            .map(Value::Module),
        Value::Module(module) => Some(Value::Module(module)),
        _ => None,
    };

    (Some(source), module)
}

/// A label with a description and details.
pub struct DynLabel {
    /// The label itself.
    pub label: Label,
    /// A description of the label.
    pub label_desc: Option<EcoString>,
    /// Additional details about the label.
    pub detail: Option<EcoString>,
    /// The title of the bibliography entry. Not present for non-bibliography
    /// labels.
    pub bib_title: Option<EcoString>,
}

/// Find all labels and details for them.
///
/// Returns:
/// - All labels and descriptions for them, if available
/// - A split offset: All labels before this offset belong to nodes, all after
///   belong to a bibliography.
#[typst_macros::time]
pub fn analyze_labels(document: &TypstDocument) -> (Vec<DynLabel>, usize) {
    let mut output = vec![];

    let _guard = GLOBAL_STATS.stat(None, "analyze_labels");

    // Labels in the document.
    for elem in document.introspector().query_labelled() {
        let Some(label) = elem.label() else { continue };
        let (is_derived, details) = {
            let derived = elem
                .get_by_name("caption")
                .or_else(|_| elem.get_by_name("body"));

            match derived {
                Ok(Value::Content(content)) => (true, content.plain_text()),
                Ok(Value::Str(s)) => (true, s.into()),
                Ok(_) => (false, elem.plain_text()),
                Err(_) => (false, elem.plain_text()),
            }
        };
        output.push(DynLabel {
            label,
            label_desc: Some(if is_derived {
                details.clone()
            } else {
                eco_format!("{}(..)", elem.func().name())
            }),
            detail: Some(details),
            bib_title: None,
        });
    }

    let split = output.len();

    // Bibliography keys.
    for (label, detail) in BibliographyElem::keys(document.introspector().track()) {
        output.push(DynLabel {
            label,
            label_desc: detail.clone(),
            detail: detail.clone(),
            bib_title: detail,
        });
    }

    (output, split)
}
