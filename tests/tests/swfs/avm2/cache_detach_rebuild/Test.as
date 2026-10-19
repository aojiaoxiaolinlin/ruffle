package {
    import flash.display.BitmapData;
    import flash.display.Sprite;
    import flash.events.Event;

    public class Test extends Sprite {
        private var cached:Sprite = new Sprite();
        private var inner:Sprite = new Sprite();
        private var frame:int = 0;

        public function Test() {
            cached.cacheAsBitmap = true;
            inner.cacheAsBitmap = true;
            paint(0xff0000);
            cached.addChild(inner);
            addChild(cached);
            addEventListener(Event.ENTER_FRAME, tick);
        }

        private function paint(color:uint):void {
            inner.graphics.clear();
            inner.graphics.beginFill(color);
            inner.graphics.drawRect(0, 0, 16, 16);
            inner.graphics.endFill();
        }

        private function sample(label:String):void {
            var image:BitmapData = new BitmapData(16, 16, false, 0);
            image.draw(cached);
            trace(label + ": " + image.getPixel(4, 4).toString(16));
            image.dispose();
        }

        private function tick(event:Event):void {
            frame++;
            if (frame == 2) {
                sample("cached");
                removeChild(cached);
                trace("preferences: " + cached.cacheAsBitmap + " " + inner.cacheAsBitmap);
                paint(0x00ff00);
                sample("detached");
                addChild(cached);
            } else if (frame == 4) {
                sample("reattached");
                removeEventListener(Event.ENTER_FRAME, tick);
            }
        }
    }
}
