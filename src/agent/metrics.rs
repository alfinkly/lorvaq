//! The two numbers lorvaq wants from gitlab-runner's Prometheus metrics: `concurrent` of the
//! process and `limit` of each registration (by short token).

use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunnerMetrics {
    pub concurrent: Option<u32>,
    /// Short runner token → `limit`; registrations without a limit (0) are left out.
    pub limits: BTreeMap<String, u32>,
}

fn value(rest: &str) -> Option<u32> {
    let number: f64 = rest.split_whitespace().next()?.parse().ok()?;
    (number.is_finite() && number >= 0.0 && number <= u32::MAX as f64).then_some(number as u32)
}

/// `name{a="x",b="y"}` → the value of label `key`.
fn label<'a>(labels: &'a str, key: &str) -> Option<&'a str> {
    labels.split(',').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"'))
    })
}

pub fn parse(text: &str) -> RunnerMetrics {
    let mut out = RunnerMetrics::default();
    for line in text.lines().map(str::trim) {
        if line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("gitlab_runner_concurrent") {
            if rest.starts_with([' ', '{']) {
                let rest = rest.rsplit_once('}').map_or(rest, |(_, r)| r);
                out.concurrent = value(rest).filter(|c| *c > 0).or(out.concurrent);
            }
        } else if let Some(rest) = line.strip_prefix("gitlab_runner_limit{") {
            let Some((labels, rest)) = rest.split_once('}') else {
                continue;
            };
            if let (Some(token), Some(limit)) = (label(labels, "runner"), value(rest)) {
                if limit > 0 && !token.is_empty() {
                    out.limits.insert(token.to_string(), limit);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_and_limits() {
        let text = r#"
# HELP gitlab_runner_concurrent The current value of concurrent setting
# TYPE gitlab_runner_concurrent gauge
gitlab_runner_concurrent 10
# TYPE gitlab_runner_limit gauge
gitlab_runner_limit{runner="abcd1234",system_id="s_0123"} 4
gitlab_runner_limit{runner="efgh5678",system_id="s_0123"} 0
gitlab_runner_limits_total 3
gitlab_runner_jobs{runner="abcd1234",state="running"} 2
"#;
        let m = parse(text);
        assert_eq!(m.concurrent, Some(10));
        assert_eq!(m.limits, BTreeMap::from([("abcd1234".to_string(), 4)]));
    }

    #[test]
    fn nothing_useful_is_empty() {
        assert_eq!(parse(""), RunnerMetrics::default());
        assert_eq!(parse("gitlab_runner_concurrent nope\n").concurrent, None);
        assert_eq!(
            parse("gitlab_runner_concurrent 1e3\n").concurrent,
            Some(1000)
        );
    }
}
