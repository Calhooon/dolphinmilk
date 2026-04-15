//! Fleet status aggregation — collect and summarize fleet member state.

use chrono::{DateTime, Utc};

use super::types::{AgentStatus, FleetMember, FleetStatus};

/// Duration after which a member with no heartbeat is marked Unknown.
const HEARTBEAT_TIMEOUT_SECS: i64 = 300; // 5 minutes

/// Aggregate fleet member statuses into a single `FleetStatus` summary.
///
/// Members whose last heartbeat exceeds `HEARTBEAT_TIMEOUT_SECS` from `now`
/// are automatically marked as `AgentStatus::Unknown`.
pub fn aggregate_fleet_status(members: &[FleetMember], now: DateTime<Utc>) -> FleetStatus {
    let mut result_members: Vec<FleetMember> = Vec::with_capacity(members.len());
    let mut total_budget: u64 = 0;
    let mut total_spent: u64 = 0;
    let mut active_tasks: usize = 0;

    for member in members {
        let mut m = member.clone();

        // Mark as Unknown if heartbeat timed out (only for non-Stopped members)
        if m.status != AgentStatus::Stopped {
            let elapsed = now.signed_duration_since(m.last_heartbeat).num_seconds();
            if elapsed > HEARTBEAT_TIMEOUT_SECS {
                m.status = AgentStatus::Unknown;
            }
        }

        // Accumulate budget totals (remaining + spent = total allocated)
        total_budget = total_budget.saturating_add(m.budget_remaining_sats);
        total_budget = total_budget.saturating_add(m.budget_spent_sats);
        total_spent = total_spent.saturating_add(m.budget_spent_sats);

        // Count active tasks
        if m.status == AgentStatus::Running && m.current_task.is_some() {
            active_tasks += 1;
        }

        result_members.push(m);
    }

    FleetStatus {
        members: result_members,
        total_budget_sats: total_budget,
        total_spent_sats: total_spent,
        active_tasks,
    }
}

/// Format a `FleetStatus` as a human-readable string for display.
pub fn format_fleet_status(status: &FleetStatus) -> String {
    let mut lines = Vec::new();

    lines.push(format!(
        "Fleet: {} members, {} active tasks",
        status.members.len(),
        status.active_tasks,
    ));
    lines.push(format!(
        "Budget: {} sats allocated, {} sats spent ({} sats remaining)",
        status.total_budget_sats,
        status.total_spent_sats,
        status
            .total_budget_sats
            .saturating_sub(status.total_spent_sats),
    ));
    lines.push(String::new());

    if status.members.is_empty() {
        lines.push("No fleet members.".to_string());
    } else {
        for member in &status.members {
            let task_str = member.current_task.as_deref().unwrap_or("none");
            lines.push(format!(
                "  {} ({}) [{}] - status: {}, task: {}, budget: {}/{} sats",
                member.name,
                &member.agent_id[..8],
                member.template,
                member.status,
                task_str,
                member.budget_remaining_sats,
                member.budget_remaining_sats + member.budget_spent_sats,
            ));
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    /// Helper to create a test member with sensible defaults.
    fn make_member(
        name: &str,
        status: AgentStatus,
        task: Option<&str>,
        remaining: u64,
        spent: u64,
        heartbeat: DateTime<Utc>,
    ) -> FleetMember {
        FleetMember {
            agent_id: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce"
                .to_string(),
            name: name.to_string(),
            template: "researcher".to_string(),
            status,
            current_task: task.map(|t| t.to_string()),
            budget_remaining_sats: remaining,
            budget_spent_sats: spent,
            last_heartbeat: heartbeat,
            endpoint: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce"
                .to_string(),
        }
    }

    #[test]
    fn test_empty_fleet_status() {
        let now = Utc::now();
        let status = aggregate_fleet_status(&[], now);

        assert!(status.members.is_empty());
        assert_eq!(status.total_budget_sats, 0);
        assert_eq!(status.total_spent_sats, 0);
        assert_eq!(status.active_tasks, 0);
    }

    #[test]
    fn test_single_running_member() {
        let now = Utc::now();
        let members = vec![make_member(
            "agent-1",
            AgentStatus::Running,
            Some("Researching fees"),
            45000,
            5000,
            now,
        )];

        let status = aggregate_fleet_status(&members, now);

        assert_eq!(status.members.len(), 1);
        assert_eq!(status.total_budget_sats, 50000); // 45000 + 5000
        assert_eq!(status.total_spent_sats, 5000);
        assert_eq!(status.active_tasks, 1);
    }

    #[test]
    fn test_running_member_without_task_not_counted_active() {
        let now = Utc::now();
        let members = vec![make_member(
            "agent-1",
            AgentStatus::Running,
            None, // Running but no task
            45000,
            5000,
            now,
        )];

        let status = aggregate_fleet_status(&members, now);

        assert_eq!(status.active_tasks, 0);
    }

    #[test]
    fn test_mixed_health_states() {
        let now = Utc::now();
        let members = vec![
            make_member(
                "runner",
                AgentStatus::Running,
                Some("Task A"),
                40000,
                10000,
                now,
            ),
            make_member("idler", AgentStatus::Idle, None, 50000, 0, now),
            make_member(
                "broken",
                AgentStatus::Error("connection refused".into()),
                None,
                20000,
                30000,
                now,
            ),
            make_member("stopped", AgentStatus::Stopped, None, 0, 50000, now),
        ];

        let status = aggregate_fleet_status(&members, now);

        assert_eq!(status.members.len(), 4);
        assert_eq!(status.active_tasks, 1); // only "runner"
        assert_eq!(status.total_spent_sats, 90000); // 10000 + 0 + 30000 + 50000
                                                    // Total budget = sum of (remaining + spent) for each member
                                                    // 50000 + 50000 + 50000 + 50000 = 200000
        assert_eq!(status.total_budget_sats, 200000);
    }

    #[test]
    fn test_heartbeat_timeout_marks_unknown() {
        let now = Utc::now();
        let stale = now - Duration::seconds(600); // 10 minutes ago

        let members = vec![
            make_member(
                "fresh",
                AgentStatus::Running,
                Some("Task"),
                40000,
                10000,
                now,
            ),
            make_member(
                "stale",
                AgentStatus::Running,
                Some("Old task"),
                30000,
                20000,
                stale,
            ),
        ];

        let status = aggregate_fleet_status(&members, now);

        assert_eq!(status.members[0].status, AgentStatus::Running);
        assert_eq!(status.members[1].status, AgentStatus::Unknown);
        // Only the fresh member counts as active
        assert_eq!(status.active_tasks, 1);
    }

    #[test]
    fn test_stopped_member_not_marked_unknown_on_stale_heartbeat() {
        let now = Utc::now();
        let stale = now - Duration::seconds(600);

        let members = vec![make_member(
            "stopped-agent",
            AgentStatus::Stopped,
            None,
            0,
            50000,
            stale,
        )];

        let status = aggregate_fleet_status(&members, now);

        // Stopped agents stay stopped even with stale heartbeat
        assert_eq!(status.members[0].status, AgentStatus::Stopped);
    }

    #[test]
    fn test_budget_aggregation() {
        let now = Utc::now();
        let members = vec![
            make_member("a", AgentStatus::Idle, None, 100000, 25000, now),
            make_member("b", AgentStatus::Idle, None, 75000, 50000, now),
            make_member("c", AgentStatus::Idle, None, 50000, 100000, now),
        ];

        let status = aggregate_fleet_status(&members, now);

        assert_eq!(status.total_spent_sats, 175000); // 25000 + 50000 + 100000
                                                     // Total budget = (100000+25000) + (75000+50000) + (50000+100000) = 400000
        assert_eq!(status.total_budget_sats, 400000);
    }

    #[test]
    fn test_format_empty_fleet() {
        let status = FleetStatus {
            members: vec![],
            total_budget_sats: 0,
            total_spent_sats: 0,
            active_tasks: 0,
        };

        let output = format_fleet_status(&status);
        assert!(output.contains("0 members"));
        assert!(output.contains("No fleet members"));
    }

    #[test]
    fn test_format_fleet_with_members() {
        let now = Utc::now();
        let status = FleetStatus {
            members: vec![FleetMember {
                agent_id: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce"
                    .to_string(),
                name: "researcher-1".to_string(),
                template: "researcher".to_string(),
                status: AgentStatus::Running,
                current_task: Some("Analyzing fees".to_string()),
                budget_remaining_sats: 45000,
                budget_spent_sats: 5000,
                last_heartbeat: now,
                endpoint: "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce"
                    .to_string(),
            }],
            total_budget_sats: 50000,
            total_spent_sats: 5000,
            active_tasks: 1,
        };

        let output = format_fleet_status(&status);
        assert!(output.contains("1 members"));
        assert!(output.contains("1 active tasks"));
        assert!(output.contains("researcher-1"));
        assert!(output.contains("Analyzing fees"));
        assert!(output.contains("45000"));
    }

    #[test]
    fn test_heartbeat_at_exact_boundary_not_unknown() {
        let now = Utc::now();
        // Exactly at the timeout boundary — should NOT be marked unknown
        let at_boundary = now - Duration::seconds(HEARTBEAT_TIMEOUT_SECS);

        let members = vec![make_member(
            "boundary",
            AgentStatus::Running,
            Some("Task"),
            40000,
            10000,
            at_boundary,
        )];

        let status = aggregate_fleet_status(&members, now);
        assert_eq!(status.members[0].status, AgentStatus::Running);
    }

    #[test]
    fn test_heartbeat_one_second_past_boundary_is_unknown() {
        let now = Utc::now();
        let past_boundary = now - Duration::seconds(HEARTBEAT_TIMEOUT_SECS + 1);

        let members = vec![make_member(
            "past",
            AgentStatus::Idle,
            None,
            40000,
            10000,
            past_boundary,
        )];

        let status = aggregate_fleet_status(&members, now);
        assert_eq!(status.members[0].status, AgentStatus::Unknown);
    }
}
