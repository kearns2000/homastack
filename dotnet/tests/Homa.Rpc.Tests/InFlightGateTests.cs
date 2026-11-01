using Homa.Rpc;

namespace Homa.Rpc.Tests;

public class InFlightGateTests
{
    [Fact]
    public async Task Work_runs_off_the_caller_and_the_cap_blocks_the_next_slot()
    {
        var gate = new RpcInFlightGate(1);
        Assert.True(await gate.WaitAsync(CancellationToken.None));

        var started = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var release = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var caller = Environment.CurrentManagedThreadId;
        var worker = 0;
        gate.Run(async () =>
        {
            worker = Environment.CurrentManagedThreadId;
            started.TrySetResult();
            await release.Task;
        });

        await started.Task.WaitAsync(TimeSpan.FromSeconds(2));
        Assert.NotEqual(caller, worker);

        var second = gate.WaitAsync(CancellationToken.None).AsTask();
        await Task.Delay(50);
        Assert.False(second.IsCompleted);

        release.TrySetResult();
        Assert.True(await second.WaitAsync(TimeSpan.FromSeconds(2)));
        gate.Release();
        await gate.DrainAsync();
    }

    [Fact]
    public void Rejects_a_non_positive_cap()
    {
        Assert.Throws<ArgumentOutOfRangeException>(() => new RpcInFlightGate(0));
        Assert.Throws<ArgumentOutOfRangeException>(() => new HomaRpcServer(Echo.Pipeline(), maxInFlight: 0));
    }
}
