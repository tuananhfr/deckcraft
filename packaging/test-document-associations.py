"""Run with python3 packaging/test-document-associations.py."""
from pathlib import Path
import unittest
import xml.etree.ElementTree as ET


ROOT = Path(__file__).resolve().parents[1]
EXTENSIONS = {"deckcraft", "pptx", "potx", "ppsx"}
OWN_EXTENSION = "deckcraft"
NS = "{http://wixtoolset.org/schemas/v4/wxs}"
CAPABILITIES = r"Software\ERPCons\TrinhChieu\Capabilities"


class DocumentAssociations(unittest.TestCase):
    def setUp(self):
        self.tree = ET.parse(ROOT / "packaging/windows/deckcraft.wxs")
        self.values = [node.attrib for node in self.tree.iter(NS + "RegistryValue")]

    def assert_registry(self, key, name, value):
        self.assertTrue(any(v["Root"] == "HKLM" and v["Key"] == key and v.get("Name") == name
                            and v["Value"] == value and v["Type"] == "string" for v in self.values),
                        (key, name, value))

    def test_windows_capabilities(self):
        self.assert_registry(CAPABILITIES, "ApplicationName", "Trình chiếu – ERPCons")
        self.assert_registry(CAPABILITIES, "ApplicationDescription", "Create and edit presentations and slide shows.")
        self.assert_registry(CAPABILITIES, "ApplicationIcon", '"[#DeckcraftExe]",0')
        self.assert_registry(r"Software\RegisteredApplications", "Trình chiếu – ERPCons", CAPABILITIES)
        associations = {v["Name"]: v["Value"] for v in self.values if v["Key"] == CAPABILITIES + r"\FileAssociations"}
        self.assertEqual(associations, {"." + ext: "ERPCons.TrinhChieu." + ext for ext in EXTENSIONS})
        supported = {v["Name"] for v in self.values if v["Key"] == r"Software\Classes\Applications\deckcraft.exe\SupportedTypes"}
        self.assertEqual(supported, set(associations))

    def test_windows_candidates_and_icons(self):
        own = self.tree.find(".//" + NS + "ProgId[@Id='ERPCons.TrinhChieu.deckcraft']")
        self.assertEqual((own.get("Icon"), own.get("IconIndex")), ("DeckcraftExe", "0"))
        self.assertIsNotNone(own.find(NS + "Extension[@Id='deckcraft']/" + NS + "Verb[@TargetFile='DeckcraftExe']"))
        for ext in EXTENSIONS - {OWN_EXTENSION}:
            with self.subTest(extension=ext):
                prog_id = "ERPCons.TrinhChieu." + ext
                self.assert_registry("Software\\Classes\\." + ext + r"\OpenWithProgids", prog_id, "")
                self.assert_registry("Software\\Classes\\" + prog_id + r"\DefaultIcon", None, '"[#DeckcraftExe]",0')
                self.assert_registry("Software\\Classes\\" + prog_id + r"\shell\open\command", None, '"[#DeckcraftExe]" "%1"')
        app = self.tree.find(".//" + NS + "File[@Id='DeckcraftExe']")
        self.assertIsNotNone(app)
        self.assertEqual(app.get("KeyPath"), "yes")
        self.assertEqual(app.get("Source"), r"$(var.BinDir)\deckcraft.exe")

    def test_windows_never_assigns_defaults(self):
        # WiX Extension elements write the extension's default; only the app's own format may have one.
        self.assertEqual([node.get("Id") for node in self.tree.iter(NS + "Extension")], [OWN_EXTENSION])
        for node in self.tree.iter():
            if node.tag not in {NS + "RegistryValue", NS + "RegistryKey"}:
                continue
            key = node.get("Key", "").lower()
            with self.subTest(key=key):
                self.assertNotIn("userchoice", key)
                if node.tag == NS + "RegistryValue":
                    if node.get("Root") == "HKCR":
                        classes_key = key
                    else:
                        classes_key = key.removeprefix("software\\classes\\")
                    self.assertFalse(classes_key.startswith(".") and "\\" not in classes_key and not node.get("Name"))


if __name__ == "__main__":
    unittest.main()
