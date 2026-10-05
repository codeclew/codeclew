namespace Asp.Versioning;

// Fixture-local stand-in with the Asp.Versioning metadata name, so the fixture
// restores without packages.
[AttributeUsage(AttributeTargets.Class | AttributeTargets.Method, AllowMultiple = true)]
public sealed class ApiVersionAttribute(string version) : Attribute
{
    public string Version { get; } = version;
}
