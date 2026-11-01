namespace Homa;

internal static class PayloadRules
{
    internal static void Check(ReadOnlySpan<byte> payload)
    {
        if (payload.IsEmpty)
        {
            throw new HomaException(
                HomaStatus.InvalidArgument,
                0,
                "Homa payloads must not be empty.");
        }

        if (payload.Length > HomaSocket.MaxPayloadLength)
        {
            throw new HomaException(
                HomaStatus.MessageTooLarge,
                0,
                $"payload is {payload.Length} bytes; the maximum is {HomaSocket.MaxPayloadLength}.");
        }
    }
}
