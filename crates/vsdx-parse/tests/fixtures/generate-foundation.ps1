param(
    [string]$OutputPath = (Join-Path $PSScriptRoot 'foundation.vsdx')
)

Add-Type -AssemblyName System.IO.Compression.FileSystem

$parts = [ordered]@{
    '[Content_Types].xml' = "<Types xmlns='http://schemas.openxmlformats.org/package/2006/content-types'><Default Extension='xml' ContentType='application/xml'/><Default Extension='rels' ContentType='application/vnd.openxmlformats-package.relationships+xml'/><Override PartName='/visio/document.xml' ContentType='application/vnd.ms-visio.drawing.main+xml'/></Types>"
    '_rels/.rels' = "<Relationships xmlns='http://schemas.openxmlformats.org/package/2006/relationships'><Relationship Id='rId1' Type='http://schemas.microsoft.com/visio/2010/relationships/document' Target='visio/document.xml'/></Relationships>"
    'visio/document.xml' = "<VisioDocument xmlns='http://schemas.microsoft.com/office/visio/2012/main'><DocumentSettings/><Colors><ColorEntry IX='0' RGB='#FFFFFF'/></Colors><FaceNames><FaceName ID='0' Name='Calibri'/></FaceNames><StyleSheets><StyleSheet ID='0' NameU='Normal'><Cell N='LineColor' V='0'/><Cell N='FillForegnd' V='1'/></StyleSheet></StyleSheets><DocumentSheet><Cell N='PageWidth' V='8.5'/><Cell N='PageHeight' V='11'/></DocumentSheet></VisioDocument>"
    'visio/_rels/document.xml.rels' = "<Relationships xmlns='http://schemas.openxmlformats.org/package/2006/relationships'><Relationship Id='rId1' Type='http://schemas.microsoft.com/visio/2010/relationships/pages' Target='pages/pages.xml'/><Relationship Id='rId2' Type='http://schemas.microsoft.com/visio/2010/relationships/masters' Target='masters/masters.xml'/><Relationship Id='rId3' Type='http://schemas.microsoft.com/visio/2010/relationships/theme' Target='theme/theme1.xml'/><Relationship Id='rId4' Type='http://schemas.microsoft.com/visio/2010/relationships/windows' Target='windows.xml'/></Relationships>"
    'visio/pages/pages.xml' = "<Pages xmlns='http://schemas.microsoft.com/office/visio/2012/main'><Page ID='1' NameU='Page-1' Name='Page-1' r:id='rId1' xmlns:r='http://schemas.openxmlformats.org/officeDocument/2006/relationships'/></Pages>"
    'visio/pages/_rels/pages.xml.rels' = "<Relationships xmlns='http://schemas.openxmlformats.org/package/2006/relationships'><Relationship Id='rId1' Type='http://schemas.microsoft.com/visio/2010/relationships/page' Target='page1.xml'/></Relationships>"
    'visio/pages/page1.xml' = "<PageContents xmlns='http://schemas.microsoft.com/office/visio/2012/main'><Shapes><Shape ID='1' NameU='Process' Type='Shape'><Cell N='PinX' V='4'/><Cell N='PinY' V='5'/><Cell N='Width' V='2'/><Cell N='Height' V='1'/><Cell N='LineWeight' V='0.01' Del='1'/><Section N='Geometry'><Row T='RelMoveTo'><Cell N='X' V='0'/><Cell N='Y' V='0'/></Row><Row T='RelLineTo' N='LineTo'><Cell N='X' V='1'/><Cell N='Y' V='1'/></Row><Row IX='2' Del='1'/></Section><Section N='Connection'><Row T='Connection'><Cell N='X' V='0.5'/><Cell N='Y' V='0.5'/></Row></Section><Section N='Scratch' Del='1'/><Text>Step <fld IX='0'/></Text></Shape></Shapes><Connects><Connect FromSheet='1' FromCell='BeginX' FromPart='9' ToSheet='1' ToCell='PinX' ToPart='3'/></Connects></PageContents>"
    'visio/masters/masters.xml' = "<Masters xmlns='http://schemas.microsoft.com/office/visio/2012/main'><Master ID='1' NameU='Master-1' r:id='rId1' xmlns:r='http://schemas.openxmlformats.org/officeDocument/2006/relationships'/></Masters>"
    'visio/masters/_rels/masters.xml.rels' = "<Relationships xmlns='http://schemas.openxmlformats.org/package/2006/relationships'><Relationship Id='rId1' Type='http://schemas.microsoft.com/visio/2010/relationships/master' Target='master1.xml'/></Relationships>"
    'visio/masters/master1.xml' = "<MasterContents xmlns='http://schemas.microsoft.com/office/visio/2012/main'><Shapes/></MasterContents>"
    'visio/theme/theme1.xml' = "<a:theme xmlns:a='http://schemas.openxmlformats.org/drawingml/2006/main' name='Office Theme'/>"
    'visio/windows.xml' = "<Windows xmlns='http://schemas.microsoft.com/office/visio/2012/main'><Window ID='0'/></Windows>"
}

if (Test-Path -LiteralPath $OutputPath) { Remove-Item -LiteralPath $OutputPath -Force }
$archive = [System.IO.Compression.ZipFile]::Open($OutputPath, [System.IO.Compression.ZipArchiveMode]::Create)
try {
    foreach ($part in $parts.GetEnumerator()) {
        $entry = $archive.CreateEntry($part.Key)
        $writer = [System.IO.StreamWriter]::new($entry.Open(), [System.Text.UTF8Encoding]::new($false))
        try { $writer.Write($part.Value) } finally { $writer.Dispose() }
    }
} finally { $archive.Dispose() }
