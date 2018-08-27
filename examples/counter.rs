//! Counter example (simple button)

#[macro_use]
extern crate mygui;

use mygui::widget::{
    Widget, WidgetCore,
    canvas::Text,
    control::TextButton,
    event::{self, NoResponse},
    layout::WidgetLayout,
    window::Window
};

enum Message {
    None,
    Incr,
}

impl From<NoResponse> for Message {
    fn from(_: NoResponse) -> Self {
        Message::None
    }
}

struct WindowInner<B> {
    display: Text,
    button: B,
    counter: usize,
}

impl_layout!(WindowInner<B: WidgetLayout>; vlist(display, button));

impl<B: WidgetLayout> WidgetCore for WindowInner<B> {}

impl<B: Widget<Response = Message>> Widget for WindowInner<B> {
    type Response = NoResponse;
    
    fn handle(&mut self, event: event::Event) -> Self::Response {
        match_event_widget!(event;
            display => self.display.handle(event).into(),
            button => {
                match button.handle(event) {
                    Message::None => {},
                    Message::Incr => {
                        self.counter += 1;
                        self.display.set_text(self.counter.to_string());
                    }
                }
                NoResponse::None
            },
        )
    }
}


fn main() {
    let mut window = Window::new(   // construct with default state and handler
        WindowInner {
            display: Text::from("0"),
            button: TextButton::new("increment", || Message::Incr),
            counter: 0
        });
    
    window.display();
}
