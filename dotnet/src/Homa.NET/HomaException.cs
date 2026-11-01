namespace Homa;

/// <summary>
/// A failed Homa call. <see cref="Detail"/> is the thread-local message from
/// <c>libhoma_ffi</c> when the failure came from native code, or a managed
/// explanation when the wrapper rejected the call first.
/// </summary>
public sealed class HomaException : Exception
{
    /// <summary>Creates an exception for <paramref name="status"/>.</summary>
    public HomaException(HomaStatus status, int nativeErrno, string? detail, Exception? innerException = null)
        : base(Format(status, nativeErrno, detail), innerException)
    {
        Status = status;
        NativeErrno = nativeErrno;
        Detail = detail ?? "";
    }

    /// <summary>Status code. See <see cref="HomaStatus"/>.</summary>
    public HomaStatus Status { get; }

    /// <summary>Underlying errno, or 0 when the failure is not an OS error.</summary>
    public int NativeErrno { get; }

    /// <summary>Human-readable explanation. May be empty.</summary>
    public string Detail { get; }

    internal static HomaException FromNative(int status)
    {
        var errno = HomaNative.LastErrorErrno();
        var detail = HomaNative.ReadLastErrorMessage();
        if (detail.Length == 0)
            detail = HomaNative.StatusText(status);
        return new HomaException((HomaStatus)status, errno, detail);
    }

    private static string Format(HomaStatus status, int nativeErrno, string? detail)
    {
        var head = nativeErrno == 0
            ? $"{status} ({(int)status})"
            : $"{status} ({(int)status}, errno {nativeErrno})";
        return string.IsNullOrEmpty(detail) ? head : head + ": " + detail;
    }
}
