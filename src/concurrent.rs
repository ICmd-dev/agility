use std::{
    sync::{
        Arc, Mutex, OnceLock,
        mpsc::{self, Receiver, SendError, Sender},
    },
    thread,
};

pub trait Participant<Input> {
    type Output;
    fn process(&self, input: Input) -> Self::Output;
}

type OptSender<T> = Sender<Option<T>>;

pub struct Process<I, T: Participant<I>> {
    participant: T,
    sender: OptSender<I>,
    next_sender: Option<OptSender<T::Output>>,
}

impl<I, T: Participant<I>> Process<I, T> {
    pub fn new(participant: T) -> Arc<Self> {
        let (sender, receiver) = mpsc::channel::<Option<I>>();
        Arc::new(Self {
            participant,
            sender: sender.clone(),
            next_sender: None,
        })
    }
    pub fn send(&self, input: I) -> Result<(), SendError<Option<I>>> {
        self.sender.send(Some(input))
    }
    pub fn stop(&self) -> Result<(), SendError<Option<I>>> {
        self.sender.send(None)
    }
    pub fn process_loop(
        value: Arc<Mutex<T>>,
        sender: OptSender<I>,
        next_sender: Option<OptSender<T::Output>>,
    ) {
        while let 
    }
}
