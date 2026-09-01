const assert = require("node:assert/strict");
const { TextModel } = require("../ui/editor.js");

const model = new TextModel("first\nsecond\nthird", "fixture.lua");
assert.equal(model.getLineCount(), 3);
assert.equal(model.getLineContent(2), "second");
assert.deepEqual(model.getPositionAt(8), { lineNumber: 2, column: 3 });
assert.equal(model.getOffsetAt({ lineNumber: 3, column: 3 }), 15);

model.applyEdit({ start: 6, end: 12, text: "changed\nline" });
assert.equal(model.getValue(), "first\nchanged\nline\nthird");
assert.equal(model.getLineCount(), 4);
model.undo();
assert.equal(model.getValue(), "first\nsecond\nthird");
model.redo();
assert.equal(model.getValue(), "first\nchanged\nline\nthird");

model.syncExternalValue("first\nchanged!\nline\nthird");
assert.equal(model.getLineContent(2), "changed!");
assert.equal(model.getVersionId(), 5);

console.log("editor model tests passed");
