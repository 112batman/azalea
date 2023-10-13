use std::collections::HashSet;
use std::hash::Hash;

use crate::context::StringRange;

use super::{Suggestion, SuggestionValue, Suggestions};

#[derive(PartialEq, Debug)]
pub struct SuggestionsBuilder<M = ()>
where
    M: Clone + Eq + Hash,
{
    input: String,
    input_lowercase: String,
    start: usize,
    remaining: String,
    remaining_lowercase: String,
    result: HashSet<Suggestion<M>>,
}

impl SuggestionsBuilder<()> {
    pub fn new(input: &str, start: usize) -> Self {
        Self::new_with_lowercase(input, input.to_lowercase().as_str(), start)
    }

    pub fn new_with_lowercase(input: &str, input_lowercase: &str, start: usize) -> Self {
        Self {
            start,
            input: input.to_string(),
            input_lowercase: input_lowercase.to_string(),
            remaining: input[start..].to_string(),
            remaining_lowercase: input_lowercase[start..].to_string(),
            result: HashSet::new(),
        }
    }
}

impl<M: Clone + Eq + Hash> SuggestionsBuilder<M> {
    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn remaining(&self) -> &str {
        &self.remaining
    }

    pub fn remaining_lowercase(&self) -> &str {
        &self.remaining_lowercase
    }

    pub fn build(&self) -> Suggestions<M> {
        Suggestions::create(&self.input, &self.result)
    }

    pub fn suggest(mut self, text: &str) -> Self {
        if text == self.remaining {
            return self;
        }
        self.result.insert(Suggestion {
            range: StringRange::between(self.start, self.input.len()),
            value: SuggestionValue::Text(text.to_string()),
            tooltip: None,
        });
        self
    }

    pub fn suggest_with_tooltip(mut self, text: &str, tooltip: M) -> Self {
        if text == self.remaining {
            return self;
        }
        self.result.insert(Suggestion {
            range: StringRange::between(self.start, self.input.len()),
            value: SuggestionValue::Text(text.to_string()),
            tooltip: Some(tooltip),
        });
        self
    }

    pub fn suggest_integer(mut self, value: i32) -> Self {
        self.result.insert(Suggestion {
            range: StringRange::between(self.start, self.input.len()),
            value: SuggestionValue::Integer(value),
            tooltip: None,
        });
        self
    }

    pub fn suggest_integer_with_tooltip(mut self, value: i32, tooltip: M) -> Self {
        self.result.insert(Suggestion {
            range: StringRange::between(self.start, self.input.len()),
            value: SuggestionValue::Integer(value),
            tooltip: Some(tooltip),
        });
        self
    }

    #[allow(clippy::should_implement_trait)]
    pub fn add(mut self, other: SuggestionsBuilder<M>) -> Self {
        self.result.extend(other.result);
        self
    }

    pub fn create_offset(&self, start: usize) -> SuggestionsBuilder<()> {
        SuggestionsBuilder::new_with_lowercase(&self.input, &self.input_lowercase, start)
    }

    pub fn restart(&self) -> SuggestionsBuilder<()> {
        self.create_offset(self.start)
    }
}
