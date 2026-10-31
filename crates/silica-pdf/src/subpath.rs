//! The subpath of a PDF link: `[[x.pdf#page=3&height=400]]`.

/// A parsed PDF link subpath. `page` is zero-based here: a link's `page=3`
/// opens the third page as a human counts, so it parses to `Some(2)`. `rest`
/// keeps every other `key=value` pair in order, undecoded.
#[derive(Debug, Clone, PartialEq)]
pub struct Subpath {
    pub page: Option<usize>,
    pub height: Option<f32>,
    pub rest: Vec<(String, String)>,
}

/// Parse `"#page=3&height=400"`, `"page=3"`, `""` and the like. A leading `#` is
/// optional. A `page` that is not a positive integer is `None`; `height` parses as
/// a finite float, anything else is `None`. A repeated key wins last; a pair
/// without `=` has an empty value. Percent-encoding is left as written.
pub fn parse_subpath(subpath: &str) -> Subpath {
    let body = subpath.strip_prefix('#').unwrap_or(subpath);
    let mut parsed = Subpath {
        page: None,
        height: None,
        rest: Vec::new(),
    };
    for pair in body.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "page" => {
                parsed.page = value
                    .parse::<usize>()
                    .ok()
                    .filter(|&page| page >= 1)
                    .map(|page| page - 1);
            }
            "height" => {
                parsed.height = value.parse::<f32>().ok().filter(|h| h.is_finite());
            }
            _ => parsed.rest.push((key.to_string(), value.to_string())),
        }
    }
    parsed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rest(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn page_and_height() {
        let s = parse_subpath("#page=3&height=400");
        assert_eq!(s.page, Some(2));
        assert_eq!(s.height, Some(400.0));
        assert!(s.rest.is_empty());
    }

    #[test]
    fn hash_is_optional() {
        assert_eq!(parse_subpath("page=3").page, Some(2));
        assert_eq!(parse_subpath("#page=3").page, Some(2));
    }

    #[test]
    fn height_alone() {
        let s = parse_subpath("height=400");
        assert_eq!(s.page, None);
        assert_eq!(s.height, Some(400.0));
        assert_eq!(parse_subpath("height=12.5").height, Some(12.5));
    }

    #[test]
    fn page_must_be_a_positive_integer() {
        assert_eq!(parse_subpath("page=0").page, None);
        assert_eq!(parse_subpath("page=-1").page, None);
        assert_eq!(parse_subpath("page=abc").page, None);
        assert_eq!(parse_subpath("page=1").page, Some(0));
        assert_eq!(parse_subpath("page=").page, None);
        assert_eq!(parse_subpath("page").page, None);
    }

    #[test]
    fn height_must_be_a_number() {
        assert_eq!(parse_subpath("height=abc").height, None);
        assert_eq!(parse_subpath("height=").height, None);
        assert_eq!(parse_subpath("height=inf").height, None);
    }

    #[test]
    fn other_keys_go_to_rest_in_order() {
        let s = parse_subpath("#foo=bar&page=2&baz=1");
        assert_eq!(s.page, Some(1));
        assert_eq!(s.rest, rest(&[("foo", "bar"), ("baz", "1")]));
    }

    #[test]
    fn empty_inputs() {
        let empty = Subpath {
            page: None,
            height: None,
            rest: vec![],
        };
        assert_eq!(parse_subpath(""), empty);
        assert_eq!(parse_subpath("#"), empty);
        assert_eq!(parse_subpath("&&"), empty);
    }

    #[test]
    fn duplicate_keys_last_wins() {
        assert_eq!(parse_subpath("page=2&page=5").page, Some(4));
        assert_eq!(parse_subpath("page=2&page=abc").page, None);
        assert_eq!(parse_subpath("height=1&height=2").height, Some(2.0));
        let s = parse_subpath("a=1&a=2");
        assert_eq!(s.rest, rest(&[("a", "1"), ("a", "2")]));
    }

    #[test]
    fn percent_encoding_is_left_as_is() {
        let s = parse_subpath("#search=hello%20world&page=4");
        assert_eq!(s.rest, rest(&[("search", "hello%20world")]));
        assert_eq!(s.page, Some(3));
    }

    #[test]
    fn value_may_contain_equals() {
        let s = parse_subpath("q=a=b");
        assert_eq!(s.rest, rest(&[("q", "a=b")]));
    }

    #[test]
    fn keys_are_case_sensitive() {
        let s = parse_subpath("Page=3");
        assert_eq!(s.page, None);
        assert_eq!(s.rest, rest(&[("Page", "3")]));
    }
}
