use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, SendError, Sender, TryRecvError},
};
use std::thread;

pub trait Participant<Input> {
    type Output: Clone;
    fn process(&self, input: Input) -> Self::Output;
}

impl<F, Input, Output> Participant<Input> for F
where
    F: Fn(Input) -> Output,
    Output: Clone,
{
    type Output = Output;
    fn process(&self, input: Input) -> Self::Output {
        (self)(input)
    }
}

struct NextStage<O> {
    sender: Sender<Option<O>>,
    thread: thread::Thread,
}

impl<O> NextStage<O> {
    fn send(&self, value: Option<O>) {
        let _ = self.sender.send(value);
        self.thread.unpark();
    }
}

pub struct Pipeline<I, O: Clone> {
    sender: Sender<Option<I>>,
    next_stages: Arc<Mutex<Vec<NextStage<O>>>>,
    processing_thread: thread::Thread,
    busy: Arc<AtomicBool>,
    child_busy: Option<Arc<AtomicBool>>,
}

impl<I, O: Clone> Pipeline<I, O> {
    pub fn new<T: Participant<I, Output = O>>(participant: T) -> Self
    where
        I: Send + 'static,
        T: Send + 'static,
        O: Send + 'static,
    {
        let (sender, receiver) = mpsc::channel();
        let (thread_tx, thread_rx) = mpsc::channel();

        let next_stages = Arc::new(Mutex::new(Vec::new()));
        let next_stages_clone = next_stages.clone();

        let busy = Arc::new(AtomicBool::new(false));
        let busy_clone = busy.clone();

        thread::spawn(move || {
            thread_tx.send(thread::current()).unwrap();
            Self::process_loop(participant, receiver, next_stages_clone, busy_clone);
        });

        Self {
            sender,
            next_stages,
            processing_thread: thread_rx.recv().unwrap(),
            busy,
            child_busy: None,
        }
    }

    pub fn connect<U: Clone>(&self, next: Pipeline<O, U>) -> Pipeline<I, U> {
        self.next_stages.lock().unwrap().push(NextStage {
            sender: next.sender.clone(),
            thread: next.processing_thread.clone(),
        });

        Pipeline {
            sender: self.sender.clone(),
            next_stages: next.next_stages,
            processing_thread: self.processing_thread.clone(),
            busy: self.busy.clone(),
            child_busy: Some(next.busy),
        }
    }

    pub fn send(&self, input: I) -> Result<(), SendError<Option<I>>> {
        let res = self.sender.send(Some(input));
        self.processing_thread.unpark();
        res
    }

    pub fn stop(&self) -> Result<(), SendError<Option<I>>> {
        let res = self.sender.send(None);
        self.processing_thread.unpark();
        res
    }

    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
            || self
                .child_busy
                .as_ref()
                .map_or(false, |b| b.load(Ordering::SeqCst))
    }

    pub fn get_busy_flag(&self) -> Arc<AtomicBool> {
        self.busy.clone()
    }

    fn process_loop<T: Participant<I, Output = O>>(
        participant: T,
        receiver: Receiver<Option<I>>,
        next_stages: Arc<Mutex<Vec<NextStage<O>>>>,
        busy: Arc<AtomicBool>,
    ) {
        loop {
            match receiver.try_recv() {
                Ok(Some(input)) => {
                    busy.store(true, Ordering::SeqCst);
                    let output = participant.process(input);
                    busy.store(false, Ordering::SeqCst);

                    for stage in next_stages.lock().unwrap().iter() {
                        stage.send(Some(output.clone()));
                    }
                }
                Ok(None) => {
                    for stage in next_stages.lock().unwrap().iter() {
                        stage.send(None);
                    }
                    break;
                }
                Err(TryRecvError::Empty) => thread::park(),
                Err(TryRecvError::Disconnected) => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_pipeline() {
        let adder = Pipeline::new(|x: u64| {
            println!("[adder] Received: {}", x);
            thread::sleep(std::time::Duration::from_millis(500));
            println!("[adder] Result: {}", x + 1);
            x + 1
        });
        let multiplier = Pipeline::new(|x: u64| {
            println!("[multiplier] Received: {}", x);
            thread::sleep(std::time::Duration::from_millis(500));
            println!("[multiplier] Result: {}", x * 2);
            x * 2
        });
        let combined = adder.connect(multiplier);
        combined.send(5).unwrap();
        combined.send(10).unwrap();
        thread::sleep(std::time::Duration::from_secs(3));
    }
}
