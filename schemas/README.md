# Vendored DCP XSD schemas

SMPTE / DCI Interop XSDs used by `schema::check_schema` for `xml_schema_violation`
validation of ASSETMAP / CPL / PKL documents. Schema validation runs by default
when this directory is present; `DCPDOCTOR_SCHEMA_DIR` overrides the location, and
a missing directory degrades to skip (so XML-only / wasm contexts still work).

## Origin

Copied verbatim from the [ClairMeta](https://github.com/ClairMeta/ClairMeta) xsd
set (`clairmeta/xsd/`), including `catalog.xml` which maps the schemas' http import
URLs to the local files so xmllint resolves them offline (xmldsig, xml.xsd, etc.).
The filenames match the namespace -> schema mapping in `schema.rs`.

## License

ClairMeta is LGPL-3.0. The underlying schemas are SMPTE / DCI Interop documents
redistributed by that project; they are not fetched from smpte-ra.org here.
