use std::{
    borrow::Cow,
    collections::{HashMap, VecDeque},
    fmt,
    time::Instant,
};

use anyhow::{Result, bail};
use downcast_rs::Downcast;
use futures::channel::oneshot::{self, Canceled, Receiver, Sender};
use lapiz_runtime::{
    Runtime,
    global::{Global, Globals},
    plugin::Plugin,
};
use lapiz_utils::{Deref, DerefMut, log_err::LogErr as _};
use tracing::info;
use uuid::Uuid;

/// Plugin that initializes the global [`UndoStacks`] resource in [`Runtime`].
pub struct UndoPlugin;

impl Plugin for UndoPlugin {
    fn build(&self, app: &mut Runtime) {
        app.add_global::<UndoStacks>();
    }
}

/// Reservation handle for an asynchronous [`UndoCommand`] that preserves submission order in an [`UndoStack`].
///
/// Created via [`UndoStack::queue`] when an operation begins asynchronously (for example, waiting on
/// a GPU readback) so that its resulting [`UndoCommand`] is committed in the exact sequence it was
/// initiated. Calling [`send`](QueuedUndoCommand::send) delivers the finished command and polls the stack.
pub struct QueuedUndoCommand {
    stack_id: Uuid,
    tx: Sender<Box<dyn UndoCommand>>,
}

impl QueuedUndoCommand {
    pub fn send(self, cmd: Box<dyn UndoCommand>, globals: &mut Globals) -> Result<()> {
        let _ = self.tx.send(cmd);
        globals.update_global::<UndoStacks, _>(|stacks, globals| {
            if let Some(stack) = stacks.get_mut(&self.stack_id) {
                stack.poll(globals)
            } else {
                bail!("No undo stack found for id: {}", self.stack_id)
            }
        })
    }
}

/// Global collection mapping canvas or document [`Uuid`]s to their respective [`UndoStack`]s.
#[derive(Default, Deref, DerefMut)]
pub struct UndoStacks {
    stacks: HashMap<Uuid, UndoStack>,
}

impl Global for UndoStacks {}

/// Bounded undo/redo command history and asynchronous ordering queue for a single document or canvas.
///
/// `UndoStack` maintains a linear history of executed [`UndoCommand`]s up to `max_history` entries
/// alongside a cursor tracking the current state.
///
/// # Command Execution & Ordering
///
/// - **Immediate Push**: [`push`](UndoStack::push) executes [`UndoCommand::redo`] immediately if no
///   asynchronous commands are pending, truncating any redo history beyond the current cursor.
/// - **Cancellation**: If a newly pushed command reports [`UndoCommand::can_cancel_out`] with the
///   command at the top of the stack, both commands are annihilated after applying the new command.
/// - **Queued Push**: [`queue`](UndoStack::queue) reserves a slot in the FIFO pending queue and
///   returns a [`QueuedUndoCommand`]. Subsequent pushes wait behind pending reservations until
///   [`poll`](UndoStack::poll) drains ready commands in order.
pub struct UndoStack {
    id: Uuid,
    cursor: usize,
    history: VecDeque<UndoCommandData>,
    queue: VecDeque<Receiver<Box<dyn UndoCommand>>>,
    max_history: usize,
}

impl UndoStack {
    pub fn new(id: Uuid, max_history: usize) -> Self {
        Self {
            id,
            cursor: 0,
            history: VecDeque::new(),
            queue: VecDeque::new(),
            max_history,
        }
    }

    pub fn queue(&mut self) -> QueuedUndoCommand {
        let (tx, rx) = oneshot::channel();
        self.queue.push_back(rx);
        QueuedUndoCommand {
            stack_id: self.id,
            tx,
        }
    }

    pub fn push<C: UndoCommand>(&mut self, cmd: C, globals: &mut Globals) -> anyhow::Result<()> {
        self.push_boxed(Box::new(cmd), globals)
    }

    pub fn push_boxed(
        &mut self,
        cmd: Box<dyn UndoCommand>,
        globals: &mut Globals,
    ) -> anyhow::Result<()> {
        if self.queue.is_empty() {
            self.push_internal(cmd, globals)?;
        } else {
            let (tx, rx) = oneshot::channel();
            self.queue.push_back(rx);
            let _ = tx.send(cmd);
            self.poll(globals)?;
        }

        Ok(())
    }

    fn push_internal(
        &mut self,
        mut cmd: Box<dyn UndoCommand>,
        globals: &mut Globals,
    ) -> anyhow::Result<()> {
        info!("Push command {}", cmd.label());
        self.history.truncate(self.cursor);

        if let Some(rhs) = self.history.back()
            && rhs.command.can_cancel_out(cmd.as_ref())
        {
            cmd.redo(globals)?;
            self.history.pop_back();
        } else {
            if self.history.len() == self.max_history {
                self.history.pop_front();
            }

            cmd.redo(globals)?;
            self.history.push_back(UndoCommandData {
                _pushed_at: Instant::now(),
                command: cmd,
            });
        }

        self.cursor = self.len();
        Ok(())
    }

    pub fn poll(&mut self, globals: &mut Globals) -> anyhow::Result<()> {
        while let Some(first) = self.queue.front_mut() {
            let cmd = match first.try_recv() {
                Ok(Some(cmd)) => cmd,
                Ok(None) => break,
                Err(Canceled) => {
                    self.queue.pop_front();
                    continue;
                }
            };

            self.queue.pop_front();
            self.push_internal(cmd, globals)?;
        }

        Ok(())
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set_cursor(&mut self, cursor: usize, globals: &mut Globals) -> anyhow::Result<()> {
        if cursor > self.len() {
            return Err(anyhow::anyhow!(
                "cursor {} out of bounds {}",
                cursor,
                self.len()
            ));
        }

        while self.cursor < cursor {
            self.cursor += 1;

            let data = &mut self.history[self.cursor - 1];
            info!("Redo {}", data.command.label());
            data.command.redo(globals).logged_err()?;
        }
        while self.cursor > cursor {
            self.cursor -= 1;

            let data = &mut self.history[self.cursor];
            info!("Undo {}", data.command.label());
            data.command.undo(globals).logged_err()?;
        }

        Ok(())
    }

    pub fn undo(&mut self, globals: &mut Globals) -> anyhow::Result<()> {
        if self.cursor == 0 {
            return Err(anyhow::anyhow!("undo stack is empty"));
        }
        self.set_cursor(self.cursor - 1, globals)
    }

    pub fn redo(&mut self, globals: &mut Globals) -> anyhow::Result<()> {
        if self.cursor == self.len() {
            return Err(anyhow::anyhow!("undo stack reached the end"));
        }
        self.set_cursor(self.cursor + 1, globals)
    }

    pub fn len(&self) -> usize {
        self.history.len()
    }

    pub fn is_empty(&self) -> bool {
        self.history.is_empty()
    }
}

impl fmt::Debug for UndoStack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UndoStack")
            .field("cursor", &self.cursor)
            .field("len", &self.len())
            .field(
                "history",
                &self
                    .history
                    .iter()
                    .map(|data| data.command.label())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// Entry in an [`UndoStack`] pairing an executed [`UndoCommand`] with its creation timestamp.
pub struct UndoCommandData {
    _pushed_at: Instant,
    command: Box<dyn UndoCommand>,
}

/// A reversible state mutation that can be applied and rolled back by an [`UndoStack`].
///
/// Implementations encapsulate the state needed to perform an edit ([`redo`](UndoCommand::redo))
/// and restore the prior state ([`undo`](UndoCommand::undo)) using [`Globals`].
///
/// # Methods
///
/// - [`label`](UndoCommand::label): Human-readable description of the operation for history display and logging.
/// - [`redo`](UndoCommand::redo): Applies (or re-applies) the command's mutation. Called automatically
///   when the command is first pushed onto an [`UndoStack`].
/// - [`undo`](UndoCommand::undo): Reverts the mutation performed by [`redo`](UndoCommand::redo).
/// - [`can_cancel_out`](UndoCommand::can_cancel_out): Optional check allowing two consecutive inverse
///   commands to cancel each other out and be removed from history.
pub trait UndoCommand: 'static + Downcast {
    fn label(&self) -> Cow<'static, str>;
    fn redo(&mut self, globals: &mut Globals) -> anyhow::Result<()>;
    fn undo(&mut self, globals: &mut Globals) -> anyhow::Result<()>;
    fn can_cancel_out(&self, _: &dyn UndoCommand) -> bool {
        false
    }
}
downcast_rs::impl_downcast!(UndoCommand);

/// Composite [`UndoCommand`] that executes a sequence of child commands as a single atomic history step.
///
/// During [`redo`](UndoCommand::redo), child commands are applied in forward order; if any command
/// fails, previously applied children in the batch are rolled back in reverse order. During
/// [`undo`](UndoCommand::undo), child commands are reverted in reverse order with forward recovery on error.
pub struct BatchedUndoCommand {
    label: Cow<'static, str>,
    commands: Vec<Box<dyn UndoCommand>>,
}

impl BatchedUndoCommand {
    pub fn new<T: UndoCommand>(label: Cow<'static, str>, commands: Vec<T>) -> Self {
        Self {
            label,
            commands: commands.into_iter().map(|c| Box::new(c) as _).collect(),
        }
    }

    pub fn new_boxed(label: Cow<'static, str>, commands: Vec<Box<dyn UndoCommand>>) -> Self {
        Self { label, commands }
    }
}

impl UndoCommand for BatchedUndoCommand {
    fn label(&self) -> Cow<'static, str> {
        self.label.clone()
    }

    fn redo(&mut self, globals: &mut Globals) -> anyhow::Result<()> {
        let mut success = 0;
        for i in 0..self.commands.len() {
            match self.commands[i].redo(globals) {
                Ok(_) => success += 1,
                Err(err) => {
                    for i in (0..success).rev() {
                        self.commands[i].undo(globals)?;
                    }
                    return Err(err);
                }
            }
        }
        Ok(())
    }

    fn undo(&mut self, globals: &mut Globals) -> anyhow::Result<()> {
        let mut success = 0;
        for i in (0..self.commands.len()).rev() {
            match self.commands[i].undo(globals) {
                Ok(_) => success += 1,
                Err(err) => {
                    for i in self.commands.len() - success..self.commands.len() {
                        self.commands[i].redo(globals)?;
                    }
                    return Err(err);
                }
            }
        }
        Ok(())
    }
}
