"""Keep English and Han tokens and remove their unused embedding rows."""

import json
import sys
import unicodedata
from pathlib import Path

import onnx
from onnx import numpy_helper


def supported(piece):
    return all(
        char.isascii()
        or not char.isalpha()
        or unicodedata.name(char, "").startswith("CJK")
        for char in piece
    )


source, destination = (Path(arg) for arg in sys.argv[1:])
destination.mkdir(parents=True, exist_ok=True)
tokenizer = json.loads((source / "tokenizer.json").read_text(encoding="utf-8"))
assert tokenizer["model"]["type"] == "Unigram"
vocabulary = tokenizer["model"]["vocab"]
special = {entry["id"] for entry in tokenizer["added_tokens"]}
kept = [index for index, (piece, _) in enumerate(vocabulary) if index in special or supported(piece)]
new_id = {old: index for index, old in enumerate(kept)}
assert [new_id[index] for index in range(4)] == [0, 1, 2, 3]

tokenizer["model"]["vocab"] = [vocabulary[index] for index in kept]
tokenizer["model"]["unk_id"] = new_id[tokenizer["model"]["unk_id"]]
for entry in tokenizer["added_tokens"]:
    entry["id"] = new_id[entry["id"]]
for entry in tokenizer["post_processor"]["special_tokens"].values():
    entry["ids"] = [new_id[index] for index in entry["ids"]]
if tokenizer["padding"]:
    tokenizer["padding"]["pad_id"] = new_id[tokenizer["padding"]["pad_id"]]

model = onnx.load(source / "e5-small-int8.onnx")
embedding = next(weight for weight in model.graph.initializer if weight.name == "embeddings.word_embeddings.weight_quantized")
rows = numpy_helper.to_array(embedding)
assert rows.shape == (250037, 384) and len(vocabulary) == 250002
embedding.CopyFrom(numpy_helper.from_array(rows[kept], name=embedding.name))
output = destination / "e5-small-int8.onnx"
onnx.save(model, output)
onnx.checker.check_model(output)
(destination / "tokenizer.json").write_text(json.dumps(tokenizer, ensure_ascii=False, separators=(",", ":")), encoding="utf-8")
print(f"Kept {len(kept):,} of {len(vocabulary):,} tokens; {output.stat().st_size / 1_000_000:.1f} MB model")
