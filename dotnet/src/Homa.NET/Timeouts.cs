namespace Homa;

internal static class Timeouts
{
    /// <summary>
    /// Maps a managed timeout onto <c>timeout_ms</c>. Null and
    /// <see cref="Timeout.InfiniteTimeSpan"/> block. Zero is one non-blocking
    /// attempt. A positive span shorter than a millisecond becomes 1 so it
    /// does not collapse into that non-blocking attempt.
    /// </summary>
    internal static long ToMilliseconds(TimeSpan? timeout)
    {
        if (timeout is null)
            return -1;

        var value = timeout.Value;
        if (value == Timeout.InfiniteTimeSpan)
            return -1;
        if (value < TimeSpan.Zero)
        {
            throw new ArgumentOutOfRangeException(
                nameof(timeout),
                "Timeout must be zero, positive, or infinite.");
        }

        var milliseconds = (long)value.TotalMilliseconds;
        if (value > TimeSpan.Zero && milliseconds == 0)
            return 1;
        return milliseconds;
    }
}
