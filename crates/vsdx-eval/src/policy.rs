use vsdx_parse::{CellLocator, MutationGesture};

use crate::{Expr, ParseLimits, parse};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MutationOutcome {
    Allowed {
        target: CellLocator,
        formula: String,
    },
    Redirected {
        from: CellLocator,
        to: CellLocator,
        formula: String,
    },
    Refused {
        reason: String,
    },
    Unsupported {
        reason: String,
    },
}

pub trait MutationContext {
    fn current_formula(&self, locator: &CellLocator) -> Result<Option<String>, String>;
    fn resolve_reference(&self, from: &CellLocator, reference: &str)
    -> Result<CellLocator, String>;
    fn lock_enabled(&self, locator: &CellLocator, lock: &str) -> Result<bool, String>;
}

/// Applies documented UI mutation interception before a ShapeSheet formula is written.
pub fn decide(
    context: &impl MutationContext,
    locator: CellLocator,
    gesture: MutationGesture,
    formula: String,
    limits: &ParseLimits,
) -> MutationOutcome {
    let Some(lock) = lock_for(gesture) else {
        return MutationOutcome::Unsupported {
            reason: "structural edits are deferred to phase 5b-2".to_owned(),
        };
    };
    match context.lock_enabled(&locator, lock) {
        Ok(true) => {
            return MutationOutcome::Refused {
                reason: format!("{lock} protects this {} gesture", gesture_name(gesture)),
            };
        }
        Ok(false) => {}
        Err(reason) => return MutationOutcome::Unsupported { reason },
    }
    let existing = match context.current_formula(&locator) {
        Ok(value) => value,
        Err(reason) => return MutationOutcome::Unsupported { reason },
    };
    let Some(existing) = existing else {
        return MutationOutcome::Allowed {
            target: locator,
            formula,
        };
    };
    let expression = match parse(existing.trim_start_matches('='), limits) {
        Ok(expression) => expression,
        Err(error) => {
            return MutationOutcome::Unsupported {
                reason: format!("cannot inspect existing formula: {}", error.message),
            };
        }
    };
    if contains_call(&expression, "GUARD") {
        return MutationOutcome::Refused {
            reason: "GUARD protects the requested cell".to_owned(),
        };
    }
    if contains_call(&expression, "SETATREFEXPR") || contains_call(&expression, "SETATREFEVAL") {
        return MutationOutcome::Unsupported {
            reason: "SETATREFEXPR/SETATREFEVAL transformations are not implemented".to_owned(),
        };
    }
    let Expr::Call(name, arguments) = expression else {
        return MutationOutcome::Allowed {
            target: locator,
            formula,
        };
    };
    if !name.eq_ignore_ascii_case("SETATREF") {
        return MutationOutcome::Allowed {
            target: locator,
            formula,
        };
    }
    let Some(Expr::Reference(reference)) = arguments.first() else {
        return MutationOutcome::Unsupported {
            reason: "SETATREF requires a cell-reference first argument".to_owned(),
        };
    };
    if arguments.len() != 1 {
        return MutationOutcome::Unsupported {
            reason: "SETATREF set_expression handling is not implemented".to_owned(),
        };
    }
    match context.resolve_reference(&locator, reference) {
        Ok(to) => MutationOutcome::Redirected {
            from: locator,
            to,
            formula,
        },
        Err(reason) => MutationOutcome::Unsupported { reason },
    }
}

fn contains_call(expression: &Expr, wanted: &str) -> bool {
    match expression {
        Expr::Call(name, arguments) => {
            name.eq_ignore_ascii_case(wanted)
                || arguments
                    .iter()
                    .any(|argument| contains_call(argument, wanted))
        }
        Expr::Unary(expression) => contains_call(expression, wanted),
        Expr::Binary(left, _, right) => contains_call(left, wanted) || contains_call(right, wanted),
        Expr::Number(_, _) | Expr::String(_) | Expr::Reference(_) => false,
    }
}

fn lock_for(gesture: MutationGesture) -> Option<&'static str> {
    match gesture {
        MutationGesture::CellEdit => Some(""),
        MutationGesture::MoveX => Some("LockMoveX"),
        MutationGesture::MoveY => Some("LockMoveY"),
        MutationGesture::ResizeWidth => Some("LockWidth"),
        MutationGesture::ResizeHeight => Some("LockHeight"),
        MutationGesture::ResizeAspect => Some("LockAspect"),
        MutationGesture::TextEdit => Some("LockTextEdit"),
        MutationGesture::Format => Some("LockFormat"),
        MutationGesture::Delete => Some("LockDelete"),
    }
}

fn gesture_name(gesture: MutationGesture) -> &'static str {
    match gesture {
        MutationGesture::CellEdit => "cell-edit",
        MutationGesture::MoveX | MutationGesture::MoveY => "move",
        MutationGesture::ResizeWidth
        | MutationGesture::ResizeHeight
        | MutationGesture::ResizeAspect => "resize",
        MutationGesture::TextEdit => "text-edit",
        MutationGesture::Format => "format",
        MutationGesture::Delete => "delete",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;

    struct Context {
        formulas: BTreeMap<String, String>,
        locks: BTreeSet<String>,
    }

    impl MutationContext for Context {
        fn current_formula(&self, locator: &CellLocator) -> Result<Option<String>, String> {
            Ok(self.formulas.get(&locator.cell_name).cloned())
        }

        fn resolve_reference(
            &self,
            from: &CellLocator,
            reference: &str,
        ) -> Result<CellLocator, String> {
            Ok(CellLocator {
                cell_name: reference.to_owned(),
                ..from.clone()
            })
        }

        fn lock_enabled(&self, _locator: &CellLocator, lock: &str) -> Result<bool, String> {
            Ok(self.locks.contains(lock))
        }
    }

    fn locator() -> CellLocator {
        CellLocator {
            sheet: vsdx_parse::CellSheet::Page(1),
            shape_id: Some(1),
            section: None,
            row: None,
            cell_name: "Width".to_owned(),
        }
    }

    #[test]
    fn reports_unsupported_setatref_transformations() {
        let context = Context {
            formulas: [(
                "Width".to_owned(),
                "SETATREF(Target, SETATREFEXPR())".to_owned(),
            )]
            .into_iter()
            .collect(),
            locks: BTreeSet::new(),
        };
        assert!(matches!(
            decide(&context, locator(), MutationGesture::ResizeWidth, "2".to_owned(), &ParseLimits::default()),
            MutationOutcome::Unsupported { reason } if reason.contains("SETATREFEXPR")
        ));
    }

    #[test]
    fn each_lock_refuses_only_its_matching_gesture() {
        for (lock, gesture) in [
            ("LockWidth", MutationGesture::ResizeWidth),
            ("LockHeight", MutationGesture::ResizeHeight),
            ("LockMoveX", MutationGesture::MoveX),
            ("LockMoveY", MutationGesture::MoveY),
            ("LockAspect", MutationGesture::ResizeAspect),
            ("LockTextEdit", MutationGesture::TextEdit),
            ("LockFormat", MutationGesture::Format),
            ("LockDelete", MutationGesture::Delete),
        ] {
            let context = Context {
                formulas: BTreeMap::new(),
                locks: [lock.to_owned()].into_iter().collect(),
            };
            assert!(matches!(
                decide(
                    &context,
                    locator(),
                    gesture,
                    "2".to_owned(),
                    &ParseLimits::default()
                ),
                MutationOutcome::Refused { .. }
            ));
            assert!(matches!(
                decide(
                    &context,
                    locator(),
                    MutationGesture::CellEdit,
                    "2".to_owned(),
                    &ParseLimits::default()
                ),
                MutationOutcome::Allowed { .. }
            ));
        }
    }
}
