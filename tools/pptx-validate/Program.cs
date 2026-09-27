// Validates a PPTX file against the Open XML schema of Office 2021.
// Exit code 0: valid. 1: one line on stdout for each error. 2: cannot read.
using DocumentFormat.OpenXml;
using DocumentFormat.OpenXml.Packaging;
using DocumentFormat.OpenXml.Validation;

if (args.Length != 1)
{
    Console.Error.WriteLine("usage: pptx-validate FILE.pptx");
    return 2;
}
try
{
    using var document = PresentationDocument.Open(args[0], false);
    var validator = new OpenXmlValidator(FileFormatVersions.Office2021);
    var errors = validator.Validate(document).ToList();
    foreach (var error in errors)
    {
        Console.WriteLine($"{error.Part?.Uri} {error.Path?.XPath}: {error.Description}");
    }
    return errors.Count == 0 ? 0 : 1;
}
catch (Exception error)
{
    Console.Error.WriteLine(error.Message);
    return 2;
}
