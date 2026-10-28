//! Sorting the grid by a column, the way people expect: numbers inside text
//! compare as numbers ("Rev2" before "Rev10"), upper/lower case is ignored,
//! and empty cells always go last.

use crate::core::Preview;
use std::cmp::Ordering;

/// Compare two cell texts "naturally".
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let si = i;
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            let sj = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let na: String = a[si..i].iter().collect::<String>().trim_start_matches('0').to_string();
            let nb: String = b[sj..j].iter().collect::<String>().trim_start_matches('0').to_string();
            let ord = na.len().cmp(&nb.len()).then_with(|| na.cmp(&nb));
            if ord != Ordering::Equal {
                return ord;
            }
        } else {
            let (ca, cb) = (a[i].to_lowercase().next().unwrap_or(a[i]), b[j].to_lowercase().next().unwrap_or(b[j]));
            if ca != cb {
                return ca.cmp(&cb);
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

/// Does any cell in the row contain the filter text (ignoring case)?
pub fn row_matches(p: &Preview, i: usize, filter_lower: &str) -> bool {
    filter_lower.is_empty() || p.rows[i].iter().any(|(shown, _)| shown.to_lowercase().contains(filter_lower))
}

/// The order to show rows in: indexes into `p.rows`, filtered then sorted.
pub fn view_order(p: &Preview, sort_col: Option<usize>, descending: bool, filter: &str) -> Vec<usize> {
    let f = filter.trim().to_lowercase();
    let mut order: Vec<usize> = (0..p.rows.len()).filter(|&i| row_matches(p, i, &f)).collect();
    let Some(c) = sort_col.filter(|c| *c < p.columns.len()) else { return order };
    if p.index_specs.get(c).map(|s| s.is_some()).unwrap_or(false) {
        // sorting by the running number = the order they were found in
        if descending {
            order.reverse();
        }
        return order;
    }
    order.sort_by(|&x, &y| {
        let (a, b) = (p.rows[x][c].0.as_str(), p.rows[y][c].0.as_str());
        match (a.is_empty(), b.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater, // empty cells last, either direction
            (false, true) => Ordering::Less,
            _ => {
                let o = natural_cmp(a, b);
                if descending {
                    o.reverse()
                } else {
                    o
                }
            }
        }
        .then(x.cmp(&y)) // keep the original order for ties
    });
    order
}

/// One cell as shown and as copied, for display row `r`. When the rows are
/// sorted by something other than the running number, running numbers are
/// renumbered in the shown order so a paste stays in sequence.
pub fn cell(p: &Preview, order: &[usize], renumber: bool, r: usize, c: usize) -> (String, String) {
    if renumber {
        if let Some(Some(spec)) = p.index_specs.get(c) {
            let t = spec.text(spec.first + r as i64);
            return (t.clone(), t);
        }
    }
    p.rows[order[r]][c].clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::IndexSpec;

    #[test]
    fn natural() {
        let mut v = vec!["Rev10", "rev2", "Rev1", "A", "b", "10", "9", "B2", "b10"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["9", "10", "A", "b", "B2", "b10", "Rev1", "rev2", "Rev10"]);
        assert_eq!(natural_cmp("2026-03", "2026-11"), Ordering::Less);
        assert_eq!(natural_cmp("007", "7"), Ordering::Equal);
    }

    fn preview(rows: &[[&str; 2]]) -> Preview {
        Preview {
            columns: vec!["No".into(), "Name".into()],
            rows: rows.iter().map(|r| r.iter().map(|s| (s.to_string(), s.to_string())).collect()).collect(),
            files: rows.len(),
            skipped: 0,
            from_sheet: true,
            letters: vec!["A".into(), "B".into()],
            filled: vec![true, true],
            start_row: 5,
            samples: vec![],
            paths: vec![],
            index_specs: vec![Some(IndexSpec { pre: "DOC-".into(), width: 3, suf: String::new(), first: 7, plain: false }), None],
        }
    }

    #[test]
    fn sort_and_renumber() {
        let p = preview(&[["DOC-007", "beta"], ["DOC-008", ""], ["DOC-009", "Alpha"], ["DOC-010", "gamma 10"], ["DOC-011", "gamma 9"]]);
        let o = view_order(&p, Some(1), false, "");
        let names: Vec<String> = (0..o.len()).map(|r| cell(&p, &o, true, r, 1).0).collect();
        assert_eq!(names, ["Alpha", "beta", "gamma 9", "gamma 10", ""]);
        // running numbers follow the new order
        let ids: Vec<String> = (0..o.len()).map(|r| cell(&p, &o, true, r, 0).1).collect();
        assert_eq!(ids, ["DOC-007", "DOC-008", "DOC-009", "DOC-010", "DOC-011"]);
        let o = view_order(&p, Some(1), true, "");
        let names: Vec<String> = (0..o.len()).map(|r| cell(&p, &o, true, r, 1).0).collect();
        assert_eq!(names, ["gamma 10", "gamma 9", "beta", "Alpha", ""], "empty still last");
        // sorting by the running number itself just reverses
        let o = view_order(&p, Some(0), true, "");
        assert_eq!(cell(&p, &o, false, 0, 0).0, "DOC-011");
        assert_eq!(view_order(&p, None, false, ""), vec![0, 1, 2, 3, 4]);
        // filtering keeps matching rows (any column, any case), then sorts them
        assert_eq!(view_order(&p, Some(1), false, "GAMMA"), vec![4, 3]);
        assert_eq!(view_order(&p, None, false, "doc-008"), vec![1]);
    }
}
