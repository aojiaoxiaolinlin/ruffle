package {
    import flash.display.Sprite;
    public class Test extends Sprite {
        public function Test() {
            var text:String = "<root>";
            for (var i:int = 0; i < 1201; i++) {
                text += '<item id="' + i + '"/>';
            }
            text += "<other/></root>";
            var root:XML = new XML(text);
            var query:XMLList = root.item;
            var separate:XMLList = root.item;
            trace("length", query.length());
            trace("order", query[0].@id, query[600].@id, query[1200].@id);
            trace("wrapper identity", query[0] === query[0]);
            trace("elements", root.elements("item").length());
            trace("wildcard", root.*.length());
            trace("missing", root.missing.length());
            query[1].@id = "changed";
            trace("shared node", root.item[1].@id, separate[1].@id);
            query[1201] = <item id="appended"/>;
            trace("append", query.length(), root.item.length(), query[1201].@id);
            trace("separate query", separate.length());
            delete query[2];
            trace("delete", query.length(), root.item.length(), query[2].@id);
            trace("other", root.other.length());
        }
    }
}
