use async_graphql_parser::types::ConstDirective;
use async_graphql_parser::Positioned;
use crate::engine::planner::error::PlannerError;

#[derive(Clone, Debug)]
pub struct CostDirective {
    pub weight: f64,
}

impl CostDirective {
    pub fn from_directive(directive: &Positioned<ConstDirective>) -> Result<Option<Self>, PlannerError> {
        if directive.node.name.node != "cost" {
            return Ok(None);
        }

        if let Some((_, arg)) = directive.node.arguments.iter().find(|(name, _)| name.node == "weight") {
            if let async_graphql_value::ConstValue::Number(n) = &arg.node {
                if let Some(f) = n.as_f64() {
                    return Ok(Some(CostDirective { weight: f }));
                }
            }
        }
        
        Err(PlannerError::QueryParseFailure("Invalid @cost directive: missing or invalid weight".to_string()))
    }
}

#[derive(Clone, Debug)]
pub struct ListSizeDirective {
    pub assumed_size: Option<i32>,
    pub slicing_arguments: Vec<String>,
    pub require_one_slicing_argument: bool,
}

impl ListSizeDirective {
    pub fn from_directive(directive: &Positioned<ConstDirective>) -> Result<Option<Self>, PlannerError> {
        if directive.node.name.node != "listSize" {
            return Ok(None);
        }

        let mut assumed_size = None;
        let mut slicing_arguments = Vec::new();
        let mut require_one_slicing_argument = false;

        for (name, arg) in &directive.node.arguments {
            match name.node.as_str() {
                "assumedSize" => {
                    if let async_graphql_value::ConstValue::Number(n) = &arg.node {
                        assumed_size = n.as_i64().map(|v| v as i32);
                    }
                }
                "slicingArguments" => {
                    if let async_graphql_value::ConstValue::List(items) = &arg.node {
                        for item in items {
                            if let async_graphql_value::ConstValue::String(s) = item {
                                slicing_arguments.push(s.clone());
                            }
                        }
                    }
                }
                "requireOneSlicingArgument" => {
                    if let async_graphql_value::ConstValue::Boolean(b) = &arg.node {
                        require_one_slicing_argument = *b;
                    }
                }
                _ => {}
            }
        }

        Ok(Some(ListSizeDirective {
            assumed_size,
            slicing_arguments,
            require_one_slicing_argument,
        }))
    }
}
