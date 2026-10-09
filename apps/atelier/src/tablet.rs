//! Read the same native event GPUI is currently dispatching, never a cached mouse value.
#[cfg(target_os = "macos")]
pub fn current_pressure() -> Option<f32> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSEventSubtype, NSEventType};
    let mtm = MainThreadMarker::new()?;
    let event = NSApplication::sharedApplication(mtm).currentEvent()?;
    let kind = event.r#type();
    let mouse = matches!(
        kind,
        NSEventType::LeftMouseDown | NSEventType::LeftMouseDragged | NSEventType::MouseMoved
    );
    if kind == NSEventType::TabletPoint || (mouse && event.subtype() == NSEventSubtype::TabletPoint)
    {
        let pressure = event.pressure();
        return pressure.is_finite().then(|| pressure.clamp(0., 1.));
    }
    None
}
#[cfg(not(target_os = "macos"))]
pub fn current_pressure() -> Option<f32> {
    None
}

#[derive(Clone, Copy)]
pub struct TabletSample {
    pub x: f32,
    pub y: f32,
    pub pressure: f32,
}
#[cfg(target_os = "macos")]
pub struct TabletInput {
    samples: std::rc::Rc<std::cell::RefCell<Vec<TabletSample>>>,
    monitor: Option<objc2::rc::Retained<objc2::runtime::AnyObject>>,
}
#[cfg(target_os = "macos")]
impl TabletInput {
    pub fn new() -> Self {
        use objc2_app_kit::{NSEvent, NSEventMask};
        let samples = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let queue = samples.clone();
        let handler = block2::RcBlock::new(move |event: std::ptr::NonNull<NSEvent>| {
            // AppKit invokes local monitors synchronously on the main thread and owns this event.
            let event_ref = unsafe { event.as_ref() };
            if let Some(mtm) = objc2::MainThreadMarker::new() {
                if let Some(view) = event_ref.window(mtm).and_then(|w| w.contentView()) {
                    let p = event_ref.locationInWindow();
                    let pressure = event_ref.pressure();
                    if pressure.is_finite() {
                        let mut queue = queue.borrow_mut();
                        if queue.len() < 4096 {
                            queue.push(TabletSample {
                                x: p.x as f32,
                                y: (view.frame().size.height - p.y) as f32,
                                pressure: pressure.clamp(0., 1.),
                            });
                        }
                    }
                }
            }
            event.as_ptr() // Preserve the original event for normal AppKit dispatch.
        });
        let monitor = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(
                NSEventMask::TabletPoint,
                &handler,
            )
        };
        Self { samples, monitor }
    }
    pub fn drain(&self) -> Vec<TabletSample> {
        std::mem::take(&mut *self.samples.borrow_mut())
    }
}
#[cfg(target_os = "macos")]
impl Drop for TabletInput {
    fn drop(&mut self) {
        if let Some(monitor) = &self.monitor {
            // This token came directly from AppKit's monitor registration.
            unsafe {
                objc2_app_kit::NSEvent::removeMonitor(monitor);
            }
        }
    }
}
#[cfg(not(target_os = "macos"))]
pub struct TabletInput;
#[cfg(not(target_os = "macos"))]
impl TabletInput {
    pub fn new() -> Self {
        Self
    }
    pub fn drain(&self) -> Vec<TabletSample> {
        Vec::new()
    }
}
