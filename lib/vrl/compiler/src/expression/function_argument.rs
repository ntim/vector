use std::{fmt, ops::Deref};

use crate::{
    expression::Expr,
    parser::{Ident, Node},
    Parameter,
};

#[derive(Clone, Debug, PartialEq)]
pub struct FunctionArgument {
    ident: Option<Node<Ident>>,
    parameter: Option<Parameter>,
    expr: Node<Expr>,
}

impl FunctionArgument {
    pub(crate) fn new(ident: Option<Node<Ident>>, expr: Node<Expr>) -> Self {
        Self {
            ident,
            parameter: None,
            expr,
        }
    }

    #[cfg(feature = "expr-function_call")]
    pub(crate) fn keyword(&self) -> Option<&str> {
        self.ident.as_ref().map(|node| node.as_ref().as_ref())
    }

    #[cfg(feature = "expr-function_call")]
    pub(crate) fn keyword_span(&self) -> Option<crate::Span> {
        self.ident.as_ref().map(Node::span)
    }

    pub(crate) fn parameter(&self) -> Option<Parameter> {
        self.parameter
    }

    pub fn expr(&self) -> &Expr {
        self.expr.inner()
    }

    #[cfg(feature = "expr-function_call")]
    pub(crate) fn expr_span(&self) -> crate::Span {
        self.expr.span()
    }

    pub(crate) fn into_inner(self) -> Expr {
        self.expr.into_inner()
    }
}

impl fmt::Display for FunctionArgument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.expr.fmt(f)
    }
}

impl Deref for FunctionArgument {
    type Target = Node<Expr>;

    fn deref(&self) -> &Self::Target {
        &self.expr
    }
}
