// Copyright 2017 Parity Technologies (UK) Ltd.
//
// Permission is hereby granted, free of charge, to any person obtaining a
// copy of this software and associated documentation files (the "Software"),
// to deal in the Software without restriction, including without limitation
// the rights to use, copy, modify, merge, publish, distribute, sublicense,
// and/or sell copies of the Software, and to permit persons to whom the
// Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
// OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
// FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.

use futures::{prelude::*, stream::iter_ok};
use std::{cmp::Ordering, io::Error as IoError, u64};

/// Description of a query to apply on a datastore.
///
/// The various modifications of the dataset are applied in the same order as the fields (prefix,
/// filters, orders, skip, limit).
#[derive(Debug, Clone)]
pub struct Query<'a, T: 'a> {
    /// Only the keys that start with `prefix` will be returned.
    pub prefix: &'a str,
    /// Filters to apply on the results.
    pub filters: &'a [Filter<'a, T>],
    /// How to order the keys. Applied sequentially.
    pub orders: &'a [Order],
    /// Number of elements to skip from at the start of the results.
    pub skip: u64,
    /// Maximum number of elements in the results.
    pub limit: u64,
    /// Only return keys. If true, then all the `Vec`s of the data will be empty.
    pub keys_only: bool,
}

impl<'a, T> Query<'a, T> {
    pub fn new() -> Query<'a, T> {
        Query {
            prefix: "",
            filters: &[],
            orders: &[],
            skip: 0,
            limit: u64::MAX,
            keys_only: false,
        }
    }

    pub fn limit(&mut self, val: u64) -> &mut Self {
        self.limit = val;
        self
    }

    pub fn skip(&mut self, val: u64) -> &mut Self {
        self.skip = val;
        self
    }

    pub fn keys_only(&mut self, val: bool) -> &mut Self {
        self.keys_only = val;
        self
    }

    pub fn orderings(&mut self, val: &'a [Order]) -> &mut Self {
        self.orders = val;
        self
    }

    pub fn filters(&mut self, val: &'a [Filter<'a, T>]) -> &mut Self {
        self.filters = val;
        self
    }

    pub fn prefix(&mut self, val: &'a str) -> &mut Self {
        self.prefix = val;
        self
    }
}

/// A filter to apply to the results set.
#[derive(Debug, Clone)]
pub struct Filter<'a, T: 'a> {
    /// Type of filter and value to compare with.
    pub ty: FilterTy<'a, T>,
    /// Comparison operation.
    pub operation: FilterOp,
}

impl<'a, T> Filter<'a, T> {
    pub fn new(ty: FilterTy<'a, T>, op: FilterOp) -> Filter<'a, T> {
        Filter { ty, operation: op }
    }
}

/// Type of filter and value to compare with.
#[derive(Debug, Clone)]
pub enum FilterTy<'a, T: 'a> {
    /// Compare the key with a reference value.
    KeyCompare(&'a str),
    /// Compare the value with a reference value.
    ValueCompare(&'a T),
}

/// Filtering operation.
#[derive(Debug, Copy, Clone)]
pub enum FilterOp {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

/// Order in which to sort the results of a query.
#[derive(Debug, Copy, Clone)]
pub enum Order {
    /// Put the values in ascending order.
    ByValueAsc,
    /// Put the values in descending order.
    ByValueDesc,
    /// Put the keys in ascending order.
    ByKeyAsc,
    /// Put the keys in descending order.
    ByKeyDesc,
}

/// Naively applies a query on a set of results.
pub fn naive_apply_query<'a, S, V>(
    stream: S,
    query: &Query<'a, V>,
) -> impl Stream<Item = (String, V), Error = IoError> + 'a
where
    S: Stream<Item = (String, V), Error = IoError> + 'a,
    V: PartialOrd + Default + 'static,
{
    let prefix = query.prefix;
    let filters = query.filters;
    let orders = query.orders;
    let keys_only = query.keys_only;

    let prefixed = stream.filter(move |item| item.0.starts_with(prefix));

    let filtered = prefixed.filter(move |item| {
        for filter in filters {
            if !naive_filter_test(&item, filter) {
                return false;
            }
        }
        true
    });

    let ordered = if orders.is_empty() {
        EitherStream::A(filtered)
    } else {
        let ordered = filtered
            .collect()
            .and_then(move |mut collected| {
                sort(&mut collected, orders);
                Ok(iter_ok(collected.into_iter()))
            })
            .flatten_stream();
        // in futures >= 0.2 `futures::Either` implements `Stream`
        EitherStream::B(ordered)
    };

    ordered
        .map(move |mut item| {
            if keys_only {
                item.1 = Default::default()
            }
            item
        })
        .skip(query.skip)
        .take(query.limit)
}

// Sort collected entries by given order constraints.
#[inline]
fn sort<T>(collected: &mut Vec<(String, T)>, orders: &[Order])
where
    T: PartialOrd,
{
    for order in orders {
        match order {
            Order::ByValueAsc => {
                collected.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));
            }
            Order::ByValueDesc => {
                collected.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
            }
            Order::ByKeyAsc => {
                collected.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
            }
            Order::ByKeyDesc => {
                collected.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
            }
        }
    }
}

#[inline]
fn naive_filter_test<T>(entry: &(String, T), filter: &Filter<T>) -> bool
where
    T: PartialOrd,
{
    let (expected_ordering, revert_expected) = match filter.operation {
        FilterOp::Equal => (Ordering::Equal, false),
        FilterOp::NotEqual => (Ordering::Equal, true),
        FilterOp::Less => (Ordering::Less, false),
        FilterOp::GreaterOrEqual => (Ordering::Less, true),
        FilterOp::Greater => (Ordering::Greater, false),
        FilterOp::LessOrEqual => (Ordering::Greater, true),
    };

    match filter.ty {
        FilterTy::KeyCompare(ref_value) => {
            (entry.0.as_str().cmp(ref_value) == expected_ordering) != revert_expected
        }
        FilterTy::ValueCompare(ref_value) => {
            (entry.1.partial_cmp(ref_value) == Some(expected_ordering)) != revert_expected
        }
    }
}

// In futures >= 0.2 `futures::Either` implements `Stream`
enum EitherStream<A, B> {
    A(A),
    B(B),
}

impl<A, B> Stream for EitherStream<A, B>
where
    A: Stream,
    B: Stream<Item = A::Item, Error = A::Error>,
{
    type Item = A::Item;
    type Error = A::Error;

    fn poll(&mut self) -> Poll<Option<A::Item>, A::Error> {
        match self {
            EitherStream::A(a) => a.poll(),
            EitherStream::B(b) => b.poll(),
        }
    }
}
