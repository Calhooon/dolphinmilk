//! Skill composition — dependency resolution and cycle detection.
//!
//! Skills can declare `requires` and `optional` dependencies on other skills.
//! When a skill is activated, its required dependencies are auto-activated first.
//! The loader validates that required dependencies exist and detects dependency cycles.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// Dependency declarations for a skill.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillDependencies {
    /// Required skills — must exist at load time. Auto-activated when parent activates.
    pub requires: Vec<String>,
    /// Optional skills — used if present, silently ignored if absent.
    pub optional: Vec<String>,
}

impl SkillDependencies {
    /// Whether this skill has any declared dependencies.
    pub fn has_dependencies(&self) -> bool {
        !self.requires.is_empty() || !self.optional.is_empty()
    }
}

/// Error types for dependency resolution.
#[derive(Debug, Clone, PartialEq)]
pub enum DependencyError {
    /// A required dependency does not exist in the registry.
    MissingRequired { skill: String, missing: String },
    /// A dependency cycle was detected.
    Cycle {
        /// The cycle path (e.g. ["a", "b", "c", "a"]).
        path: Vec<String>,
    },
}

impl std::fmt::Display for DependencyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DependencyError::MissingRequired { skill, missing } => {
                write!(
                    f,
                    "Skill '{}' requires '{}' which does not exist",
                    skill, missing
                )
            }
            DependencyError::Cycle { path } => {
                write!(f, "Dependency cycle detected: {}", path.join(" -> "))
            }
        }
    }
}

impl std::error::Error for DependencyError {}

/// Validate that all required dependencies exist in the set of available skill names.
///
/// Returns a list of errors for any missing required dependencies.
pub fn validate_required_deps(
    skill_name: &str,
    deps: &SkillDependencies,
    available_skills: &HashSet<String>,
) -> Vec<DependencyError> {
    let mut errors = Vec::new();
    for required in &deps.requires {
        if !available_skills.contains(required) {
            errors.push(DependencyError::MissingRequired {
                skill: skill_name.to_string(),
                missing: required.clone(),
            });
        }
    }
    errors
}

/// Detect dependency cycles in the skill dependency graph.
///
/// Takes a map of skill_name -> required dependencies and returns an error if
/// any cycle is found. Uses iterative DFS with explicit stack (no recursion).
pub fn detect_cycles(
    dependency_graph: &HashMap<String, Vec<String>>,
) -> Result<(), DependencyError> {
    // States for cycle detection via 3-color DFS
    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        White, // Unvisited
        Gray,  // In current DFS path
        Black, // Fully processed
    }

    let mut colors: HashMap<&str, Color> = HashMap::new();
    for key in dependency_graph.keys() {
        colors.insert(key.as_str(), Color::White);
    }

    // For each unvisited node, run DFS
    for start in dependency_graph.keys() {
        if colors.get(start.as_str()) != Some(&Color::White) {
            continue;
        }

        // Iterative DFS using an explicit stack
        // Stack entries: (node, is_backtrack)
        let mut stack: Vec<(&str, bool)> = vec![(start.as_str(), false)];
        // Track the current DFS path for cycle reporting
        let mut path: Vec<&str> = Vec::new();

        while let Some((node, is_backtrack)) = stack.pop() {
            if is_backtrack {
                // We're returning from this node — mark it Black
                colors.insert(node, Color::Black);
                path.pop();
                continue;
            }

            let color = colors.get(node).copied().unwrap_or(Color::White);

            match color {
                Color::Gray => {
                    // Found a cycle — build the cycle path
                    let mut cycle_path: Vec<String> = path
                        .iter()
                        .skip_while(|&&n| n != node)
                        .map(|&n| n.to_string())
                        .collect();
                    cycle_path.push(node.to_string());
                    return Err(DependencyError::Cycle { path: cycle_path });
                }
                Color::Black => {
                    // Already fully processed, skip
                    continue;
                }
                Color::White => {
                    // Mark as in-progress
                    colors.insert(node, Color::Gray);
                    path.push(node);

                    // Push backtrack marker
                    stack.push((node, true));

                    // Push dependencies
                    if let Some(deps) = dependency_graph.get(node) {
                        for dep in deps.iter().rev() {
                            stack.push((dep.as_str(), false));
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Compute the activation order for a skill and all its transitive required
/// dependencies (topological sort). Returns skills in dependency-first order.
///
/// Does NOT include optional dependencies — those are informational only.
pub fn activation_order(
    skill_name: &str,
    dependency_graph: &HashMap<String, Vec<String>>,
) -> Vec<String> {
    let mut visited = HashSet::new();
    let mut order = Vec::new();
    activation_order_dfs(skill_name, dependency_graph, &mut visited, &mut order);
    order
}

fn activation_order_dfs(
    skill_name: &str,
    dependency_graph: &HashMap<String, Vec<String>>,
    visited: &mut HashSet<String>,
    order: &mut Vec<String>,
) {
    if visited.contains(skill_name) {
        return;
    }
    visited.insert(skill_name.to_string());

    // Visit dependencies first
    if let Some(deps) = dependency_graph.get(skill_name) {
        for dep in deps {
            activation_order_dfs(dep, dependency_graph, visited, order);
        }
    }

    order.push(skill_name.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_required_deps_all_present() {
        let deps = SkillDependencies {
            requires: vec!["web-search".to_string(), "memory".to_string()],
            optional: vec!["summarization".to_string()],
        };
        let available: HashSet<String> = ["web-search", "memory", "summarization"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        let errors = validate_required_deps("advanced-research", &deps, &available);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_required_deps_missing() {
        let deps = SkillDependencies {
            requires: vec!["web-search".to_string(), "missing-skill".to_string()],
            optional: vec![],
        };
        let available: HashSet<String> = ["web-search"].iter().map(|s| s.to_string()).collect();

        let errors = validate_required_deps("my-skill", &deps, &available);
        assert_eq!(errors.len(), 1);
        match &errors[0] {
            DependencyError::MissingRequired { skill, missing } => {
                assert_eq!(skill, "my-skill");
                assert_eq!(missing, "missing-skill");
            }
            _ => panic!("Expected MissingRequired"),
        }
    }

    #[test]
    fn test_validate_optional_missing_is_ok() {
        let deps = SkillDependencies {
            requires: vec![],
            optional: vec!["not-installed".to_string()],
        };
        let available: HashSet<String> = HashSet::new();

        let errors = validate_required_deps("my-skill", &deps, &available);
        assert!(errors.is_empty(), "Optional deps should not cause errors");
    }

    #[test]
    fn test_detect_cycles_no_cycle() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec!["b".to_string()]);
        graph.insert("b".to_string(), vec!["c".to_string()]);
        graph.insert("c".to_string(), vec![]);

        assert!(detect_cycles(&graph).is_ok());
    }

    #[test]
    fn test_detect_cycles_self_cycle() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec!["a".to_string()]);

        let err = detect_cycles(&graph).unwrap_err();
        match err {
            DependencyError::Cycle { path } => {
                assert!(path.contains(&"a".to_string()));
                assert!(path.len() >= 2, "Cycle path should show a -> a");
            }
            _ => panic!("Expected Cycle"),
        }
    }

    #[test]
    fn test_detect_cycles_mutual() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec!["b".to_string()]);
        graph.insert("b".to_string(), vec!["a".to_string()]);

        let err = detect_cycles(&graph).unwrap_err();
        match err {
            DependencyError::Cycle { path } => {
                assert!(path.len() >= 2);
            }
            _ => panic!("Expected Cycle"),
        }
    }

    #[test]
    fn test_detect_cycles_transitive() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec!["b".to_string()]);
        graph.insert("b".to_string(), vec!["c".to_string()]);
        graph.insert("c".to_string(), vec!["a".to_string()]);

        let err = detect_cycles(&graph).unwrap_err();
        match err {
            DependencyError::Cycle { path } => {
                assert!(path.len() >= 3, "Cycle path: {:?}", path);
            }
            _ => panic!("Expected Cycle"),
        }
    }

    #[test]
    fn test_detect_cycles_diamond_no_cycle() {
        // Diamond: a -> b, a -> c, b -> d, c -> d
        // No cycle!
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec!["b".to_string(), "c".to_string()]);
        graph.insert("b".to_string(), vec!["d".to_string()]);
        graph.insert("c".to_string(), vec!["d".to_string()]);
        graph.insert("d".to_string(), vec![]);

        assert!(detect_cycles(&graph).is_ok());
    }

    #[test]
    fn test_detect_cycles_empty_graph() {
        let graph: HashMap<String, Vec<String>> = HashMap::new();
        assert!(detect_cycles(&graph).is_ok());
    }

    #[test]
    fn test_activation_order_simple_chain() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec!["b".to_string()]);
        graph.insert("b".to_string(), vec!["c".to_string()]);
        graph.insert("c".to_string(), vec![]);

        let order = activation_order("a", &graph);
        assert_eq!(order, vec!["c", "b", "a"]);
    }

    #[test]
    fn test_activation_order_no_deps() {
        let graph: HashMap<String, Vec<String>> = HashMap::new();
        let order = activation_order("standalone", &graph);
        assert_eq!(order, vec!["standalone"]);
    }

    #[test]
    fn test_activation_order_diamond() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec!["b".to_string(), "c".to_string()]);
        graph.insert("b".to_string(), vec!["d".to_string()]);
        graph.insert("c".to_string(), vec!["d".to_string()]);
        graph.insert("d".to_string(), vec![]);

        let order = activation_order("a", &graph);
        // d must come before b and c; b and c before a
        let pos = |name: &str| order.iter().position(|s| s == name).unwrap();
        assert!(pos("d") < pos("b"));
        assert!(pos("d") < pos("c"));
        assert!(pos("b") < pos("a"));
        assert!(pos("c") < pos("a"));
    }

    #[test]
    fn test_has_dependencies() {
        let empty = SkillDependencies::default();
        assert!(!empty.has_dependencies());

        let with_req = SkillDependencies {
            requires: vec!["foo".to_string()],
            optional: vec![],
        };
        assert!(with_req.has_dependencies());

        let with_opt = SkillDependencies {
            requires: vec![],
            optional: vec!["bar".to_string()],
        };
        assert!(with_opt.has_dependencies());
    }

    #[test]
    fn test_dependency_error_display() {
        let missing = DependencyError::MissingRequired {
            skill: "research".to_string(),
            missing: "web-search".to_string(),
        };
        let msg = format!("{}", missing);
        assert!(msg.contains("research"));
        assert!(msg.contains("web-search"));
        assert!(msg.contains("does not exist"));

        let cycle = DependencyError::Cycle {
            path: vec!["a".to_string(), "b".to_string(), "a".to_string()],
        };
        let msg = format!("{}", cycle);
        assert!(msg.contains("a -> b -> a"));
    }
}
