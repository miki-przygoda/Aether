use tokio::sync::broadcast;

/// Sent to every subscriber when the panic button is pressed or the user
/// signals an abort. All active audio and gRPC stream tasks listen for this.
#[derive(Debug, Clone)]
pub struct KillSignal;

/// Create a kill-signal broadcast channel.  Capacity 4 is sufficient — there
/// are at most 3 concurrent tasks (audio stream, TTS playback, LED driver) and
/// any surplus is just dropped once all subscribers drain.
pub fn channel() -> (
    broadcast::Sender<KillSignal>,
    broadcast::Receiver<KillSignal>,
) {
    broadcast::channel(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn all_subscribers_receive_kill() {
        let (tx, _rx0) = channel();
        let mut rx1 = tx.subscribe();
        let mut rx2 = tx.subscribe();
        let mut rx3 = tx.subscribe();

        tx.send(KillSignal).unwrap();

        assert!(rx1.recv().await.is_ok());
        assert!(rx2.recv().await.is_ok());
        assert!(rx3.recv().await.is_ok());
    }

    #[tokio::test]
    async fn no_subscribers_send_does_not_panic() {
        let (tx, _rx) = channel();
        // Send with no active subscribers should not panic; the error is ignored.
        let _ = tx.send(KillSignal);
    }
}
