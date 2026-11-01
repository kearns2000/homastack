using System.Collections.Concurrent;

namespace Homa.Rpc;

/// <summary>
/// Limits how many RPC handlers run at once and moves each one off the caller.
/// The receive loop waits for a slot, then <see cref="Run"/> returns before the
/// handler finishes, so a synchronous handler does not stall the next receive.
/// </summary>
internal sealed class RpcInFlightGate
{
    private readonly SemaphoreSlim _slots;
    private readonly ConcurrentDictionary<Task, byte> _inflight = new();

    public RpcInFlightGate(int maxInFlight)
    {
        if (maxInFlight < 1)
            throw new ArgumentOutOfRangeException(nameof(maxInFlight), "At least one in-flight RPC is required.");
        MaxInFlight = maxInFlight;
        _slots = new SemaphoreSlim(maxInFlight, maxInFlight);
    }

    public int MaxInFlight { get; }

    /// <summary>Takes one slot. False when <paramref name="cancellationToken"/> is cancelled.</summary>
    public async ValueTask<bool> WaitAsync(CancellationToken cancellationToken)
    {
        try
        {
            await _slots.WaitAsync(cancellationToken).ConfigureAwait(false);
            return true;
        }
        catch (OperationCanceledException)
        {
            return false;
        }
    }

    /// <summary>Returns a slot taken with <see cref="WaitAsync"/> when no handler is started.</summary>
    public void Release() => _slots.Release();

    /// <summary>
    /// Runs <paramref name="work"/> on the thread pool and returns the slot when it finishes.
    /// The returned task is tracked until <see cref="DrainAsync"/>.
    /// </summary>
    public void Run(Func<Task> work)
    {
        ArgumentNullException.ThrowIfNull(work);
        var task = Task.Run(async () =>
        {
            try
            {
                await work().ConfigureAwait(false);
            }
            finally
            {
                _slots.Release();
            }
        });
        _inflight.TryAdd(task, 0);
        _ = task.ContinueWith(
            finished =>
            {
                _inflight.TryRemove(finished, out _);
                _ = finished.Exception;
            },
            CancellationToken.None,
            TaskContinuationOptions.ExecuteSynchronously,
            TaskScheduler.Default);
    }

    public async Task DrainAsync()
    {
        try
        {
            await Task.WhenAll(_inflight.Keys.ToArray()).ConfigureAwait(false);
        }
        catch (Exception)
        {
            // Workers report their own failures. Drain waits for them to finish.
        }
    }
}
