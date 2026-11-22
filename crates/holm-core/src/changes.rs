use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};

use futures_channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use futures_core::Stream;

use crate::lock;
use crate::model::ChangeList;

#[derive(Debug, Default)]
pub(crate) struct Listeners {
    senders: Mutex<Vec<UnboundedSender<ChangeList>>>,
}

impl Listeners {
    /// A new stream that gets each list sent from now on.
    pub(crate) fn add(&self) -> ChangeLists {
        let (send, receive) = mpsc::unbounded();
        lock(&self.senders).push(send);
        ChangeLists { receive }
    }

    /// The last stream gets `list` itself, so a long list is cloned only for the others.
    pub(crate) fn send(&self, list: ChangeList) {
        let mut senders = lock(&self.senders);
        senders.retain(|sender| !sender.is_closed());
        if let Some((last, others)) = senders.split_last() {
            for sender in others {
                // A stream that ends now is dropped at the next list.
                let _ = sender.unbounded_send(list.clone());
            }
            let _ = last.unbounded_send(list);
        }
    }
}

/// [`Core::follow`](crate::Core::follow) makes it.
///
/// It ends when every [`Core`](crate::Core) handle is gone.
#[derive(Debug)]
pub struct ChangeLists {
    receive: UnboundedReceiver<ChangeList>,
}

impl ChangeLists {
    /// The next change list, with no wait.
    ///
    /// It returns `None` when no list waits, or when every [`Core`](crate::Core) handle is gone.
    pub fn try_next(&mut self) -> Option<ChangeList> {
        self.receive.try_recv().ok()
    }
}

impl Stream for ChangeLists {
    type Item = ChangeList;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<ChangeList>> {
        Pin::new(&mut self.receive).poll_next(cx)
    }
}
