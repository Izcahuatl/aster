(function attachAsterEditor(globalScope, factory) {
  const api = factory();
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  globalScope.AsterEditor = api;
})(typeof globalThis !== "undefined" ? globalThis : window, () => {
  "use strict";

  function clamp(value, minimum, maximum) {
    return Math.max(minimum, Math.min(maximum, value));
  }

  class TextModel {
    constructor(value = "", uri = "") {
      this.uri = uri;
      this._value = String(value);
      this._lineStarts = [];
      this._version = 1;
      this._listeners = new Set();
      this._undoStack = [];
      this._redoStack = [];
      this._rebuildLineIndex();
    }

    getValue() {
      return this._value;
    }

    getVersionId() {
      return this._version;
    }

    getLineCount() {
      return this._lineStarts.length;
    }

    getLineContent(lineNumber) {
      const line = clamp(lineNumber, 1, this.getLineCount());
      const start = this._lineStarts[line - 1];
      const end = line < this.getLineCount() ? this._lineStarts[line] - 1 : this._value.length;
      return this._value.slice(start, end).replace(/\r$/, "");
    }

    getPositionAt(offset) {
      const safeOffset = clamp(offset, 0, this._value.length);
      let low = 0;
      let high = this._lineStarts.length - 1;
      while (low <= high) {
        const middle = (low + high) >>> 1;
        if (this._lineStarts[middle] <= safeOffset) low = middle + 1;
        else high = middle - 1;
      }
      const lineIndex = Math.max(0, high);
      return {
        lineNumber: lineIndex + 1,
        column: safeOffset - this._lineStarts[lineIndex] + 1,
      };
    }

    getOffsetAt(position) {
      const lineNumber = clamp(position.lineNumber, 1, this.getLineCount());
      const lineStart = this._lineStarts[lineNumber - 1];
      const lineLength = this.getLineContent(lineNumber).length;
      return lineStart + clamp(position.column - 1, 0, lineLength);
    }

    setValue(value, options = {}) {
      const nextValue = String(value);
      if (nextValue === this._value) return null;
      return this.applyEdit(
        { start: 0, end: this._value.length, text: nextValue },
        { ...options, recordHistory: options.recordHistory ?? false },
      );
    }

    applyEdit(edit, options = {}) {
      const start = clamp(edit.start, 0, this._value.length);
      const end = clamp(edit.end, start, this._value.length);
      const text = String(edit.text ?? "");
      const removedText = this._value.slice(start, end);
      if (removedText === text) return null;

      const operation = {
        start,
        removedText,
        insertedText: text,
        selectionBefore: options.selectionBefore || null,
        selectionAfter: options.selectionAfter || null,
      };
      this._replace(start, end, text, options.source || "edit", operation);

      if (options.recordHistory !== false) {
        this._undoStack.push(operation);
        this._redoStack.length = 0;
      }
      return operation;
    }

    syncExternalValue(value, options = {}) {
      const nextValue = String(value);
      if (nextValue === this._value) return null;

      let prefix = 0;
      const sharedLength = Math.min(this._value.length, nextValue.length);
      while (prefix < sharedLength && this._value[prefix] === nextValue[prefix]) prefix++;

      let oldSuffix = this._value.length;
      let newSuffix = nextValue.length;
      while (
        oldSuffix > prefix &&
        newSuffix > prefix &&
        this._value[oldSuffix - 1] === nextValue[newSuffix - 1]
      ) {
        oldSuffix--;
        newSuffix--;
      }

      return this.applyEdit(
        { start: prefix, end: oldSuffix, text: nextValue.slice(prefix, newSuffix) },
        options,
      );
    }

    undo() {
      const operation = this._undoStack.pop();
      if (!operation) return null;
      this._replace(
        operation.start,
        operation.start + operation.insertedText.length,
        operation.removedText,
        "undo",
        operation,
      );
      this._redoStack.push(operation);
      return operation.selectionBefore;
    }

    redo() {
      const operation = this._redoStack.pop();
      if (!operation) return null;
      this._replace(
        operation.start,
        operation.start + operation.removedText.length,
        operation.insertedText,
        "redo",
        operation,
      );
      this._undoStack.push(operation);
      return operation.selectionAfter;
    }

    onDidChange(listener) {
      this._listeners.add(listener);
      return () => this._listeners.delete(listener);
    }

    _replace(start, end, text, source, operation) {
      this._value = this._value.slice(0, start) + text + this._value.slice(end);
      this._version++;
      this._rebuildLineIndex();
      const event = { source, versionId: this._version, operation, value: this._value };
      this._listeners.forEach((listener) => listener(event));
    }

    _rebuildLineIndex() {
      this._lineStarts = [0];
      for (let index = 0; index < this._value.length; index++) {
        if (this._value[index] === "\n") this._lineStarts.push(index + 1);
      }
    }
  }

  class EditorView {
    constructor(textarea) {
      this.element = textarea;
      this.model = null;
      this._disposeModelListener = null;
      this._modelViewState = new WeakMap();
      this._selectionBeforeInput = { start: 0, end: 0 };
      this._characterWidth = null;
      this.element.addEventListener("beforeinput", () => {
        this._selectionBeforeInput = this.getSelection();
      });
    }

    setModel(model) {
      if (this.model) {
        this._modelViewState.set(this.model, {
          selection: this.getSelection(),
          scrollTop: this.element.scrollTop,
          scrollLeft: this.element.scrollLeft,
        });
      }
      if (this._disposeModelListener) this._disposeModelListener();

      this.model = model;
      this.element.value = model ? model.getValue() : "";
      this._disposeModelListener = model
        ? model.onDidChange(() => {
            if (this.element.value !== model.getValue()) this.element.value = model.getValue();
          })
        : null;

      const saved = model ? this._modelViewState.get(model) : null;
      const selection = saved?.selection || { start: 0, end: 0 };
      this.setSelection(selection.start, selection.end);
      this.element.scrollTop = saved?.scrollTop || 0;
      this.element.scrollLeft = saved?.scrollLeft || 0;
    }

    getText() {
      return this.model ? this.model.getValue() : "";
    }

    setText(value) {
      if (!this.model) return;
      this.model.setValue(value, { source: "set-value", recordHistory: false });
    }

    syncFromDom() {
      if (!this.model) return null;
      return this.model.syncExternalValue(this.element.value, {
        source: "input",
        selectionBefore: this._selectionBeforeInput,
        selectionAfter: this.getSelection(),
      });
    }

    getSelection() {
      return { start: this.element.selectionStart, end: this.element.selectionEnd };
    }

    setSelection(start, end = start) {
      const maximum = this.model ? this.model.getValue().length : 0;
      this.element.setSelectionRange(clamp(start, 0, maximum), clamp(end, 0, maximum));
    }

    getCursorPosition() {
      return this.model
        ? this.model.getPositionAt(this.element.selectionStart)
        : { lineNumber: 1, column: 1 };
    }

    replaceSelection(text, source = "command") {
      if (!this.model) return;
      const selection = this.getSelection();
      const caret = selection.start + text.length;
      this.model.applyEdit(
        { start: selection.start, end: selection.end, text },
        {
          source,
          selectionBefore: selection,
          selectionAfter: { start: caret, end: caret },
        },
      );
      this.setSelection(caret);
    }

    positionFromPoint(event, tabSize = 4) {
      if (!this.model) return null;
      const bounds = this.element.getBoundingClientRect();
      const style = getComputedStyle(this.element);
      const lineHeight = Number.parseFloat(style.lineHeight);
      const paddingLeft = Number.parseFloat(style.paddingLeft);
      const paddingTop = Number.parseFloat(style.paddingTop);
      if (!Number.isFinite(lineHeight) || lineHeight <= 0) return null;

      if (!this._characterWidth) {
        const canvas = document.createElement("canvas");
        const context = canvas.getContext("2d");
        context.font = `${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
        this._characterWidth = context.measureText("M").width;
      }

      const x = event.clientX - bounds.left - paddingLeft + this.element.scrollLeft;
      const y = event.clientY - bounds.top - paddingTop + this.element.scrollTop;
      if (x < 0 || y < 0 || !this._characterWidth) return null;

      const lineNumber = Math.floor(y / lineHeight) + 1;
      if (lineNumber < 1 || lineNumber > this.model.getLineCount()) return null;
      const visualColumn = Math.floor(x / this._characterWidth);
      const line = this.model.getLineContent(lineNumber);
      let sourceColumn = 0;
      let visualOffset = 0;
      while (sourceColumn < line.length && visualOffset <= visualColumn) {
        const width = line[sourceColumn] === "\t" ? tabSize - (visualOffset % tabSize) : 1;
        if (visualColumn < visualOffset + width) break;
        visualOffset += width;
        sourceColumn++;
      }
      if (visualColumn > visualOffset || sourceColumn > line.length) return null;
      return { lineNumber, column: sourceColumn + 1 };
    }

    focus() {
      this.element.focus();
    }
  }

  return { TextModel, EditorView };
});
