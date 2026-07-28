use crate::avrcp::AvrcpAction;

pub fn method_to_avrcp_action(method: &str) -> Option<AvrcpAction> {
    match method {
        "media.control.play" => Some(AvrcpAction::Play),
        "media.control.pause" => Some(AvrcpAction::Pause),
        "media.control.next" => Some(AvrcpAction::Next),
        "media.control.previous" | "media.control.prev" => Some(AvrcpAction::Previous),
        "media.control.volumeUp" => Some(AvrcpAction::VolumeUp),
        "media.control.volumeDown" => Some(AvrcpAction::VolumeDown),
        _ => None,
    }
}
