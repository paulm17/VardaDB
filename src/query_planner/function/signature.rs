//! Function signatures, ported from upstream `exec/function/signature.rs`.
//!
//! Upstream validates against SurrealDB's rich `Kind` lattice; VardaDB keeps
//! the builder shape (`arg`/`optional`/`variadic`/`returns`) but documents
//! parameter kinds as descriptive strings, since strict typing is enforced at
//! evaluation time against [`QueryValue`] variants rather than a static kind
//! system.

/// One declared parameter of a [`Signature`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub name: &'static str,
    /// Descriptive kind label ("string", "number", "any", ...).
    pub kind: &'static str,
    /// `false` for parameters introduced by [`Signature::optional`].
    pub required: bool,
}

/// Declared call shape of a scalar function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    params: Vec<Param>,
    variadic: bool,
    returns: &'static str,
}

impl Default for Signature {
    fn default() -> Self {
        Signature {
            params: Vec::new(),
            variadic: false,
            returns: "any",
        }
    }
}

impl Signature {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a required positional parameter.
    pub fn arg(mut self, name: &'static str, kind: &'static str) -> Self {
        self.params.push(Param {
            name,
            kind,
            required: true,
        });
        self
    }

    /// Append an optional parameter (optionals must trail required ones).
    pub fn optional(mut self, name: &'static str, kind: &'static str) -> Self {
        self.params.push(Param {
            name,
            kind,
            required: false,
        });
        self
    }

    /// Mark the last parameter as accepting trailing repetitions.
    pub fn variadic(mut self, _kind: &'static str) -> Self {
        self.variadic = true;
        self
    }

    /// Declare the result kind label.
    pub fn returns(mut self, kind: &'static str) -> Self {
        self.returns = kind;
        self
    }

    /// Smallest accepted argument count.
    pub fn min_args(&self) -> usize {
        self.params.iter().filter(|p| p.required).count()
    }

    /// Largest accepted argument count; `None` when variadic.
    pub fn max_args(&self) -> Option<usize> {
        if self.variadic {
            None
        } else {
            Some(self.params.len())
        }
    }

    pub fn accepts_arity(&self, n: usize) -> bool {
        n >= self.min_args() && self.max_args().is_none_or(|max| n <= max)
    }

    /// Human-readable arity range used in [`crate::query_planner::physical_expr::ExprError`].
    pub fn arity_label(&self) -> String {
        match self.max_args() {
            Some(max) => format!("{}..{}", self.min_args(), max),
            None => format!("{}..N", self.min_args()),
        }
    }
}
