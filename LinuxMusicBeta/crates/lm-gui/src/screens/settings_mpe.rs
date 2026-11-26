//! The MPE setting of a MIDI port (`decisoes-autonomas.md`, 26/11, item 5), as the settings page keeps it in the
//! port's preferences: `mpe` is `"off"`, `"lower"` or `"upper"`, `mpeMembers` the member channels of the zone.

use lm_port::Value;

const ZONES: [&str; 3] = ["off", "lower", "upper"];

/// The zone a port preference has (`off` when absent)
pub fn zone(port: &Value) -> &'static str {
    let z = port.get("mpe").and_then(Value::as_text).unwrap_or("off");
    ZONES.iter().copied().find(|k| *k == z).unwrap_or("off")
}

/// The zone after a step of the stepper (wraps around)
pub fn step_zone(port: &Value, d: i64) -> &'static str {
    let i = ZONES.iter().position(|k| *k == zone(port)).unwrap_or(0) as i64;
    ZONES[(i + d).rem_euclid(ZONES.len() as i64) as usize]
}

/// The member channels of the zone (15 when absent)
pub fn members(port: &Value) -> i64 {
    port.get("mpeMembers").and_then(Value::as_int).unwrap_or(15).clamp(1, 15)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zone_steps_around_and_the_members_default_to_all() {
        let none = Value::object([] as [(&str, Value); 0]);
        assert_eq!((zone(&none), members(&none)), ("off", 15));
        assert_eq!(step_zone(&none, 1), "lower");
        assert_eq!(step_zone(&none, -1), "upper");
        let upper = Value::object([("mpe", Value::text("upper")), ("mpeMembers", Value::int(4))]);
        assert_eq!((zone(&upper), members(&upper), step_zone(&upper, 1)), ("upper", 4, "off"));
    }
}
