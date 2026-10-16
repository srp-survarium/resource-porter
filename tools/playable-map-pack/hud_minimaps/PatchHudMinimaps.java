// SPDX-License-Identifier: GPL-3.0-or-later
// Add minimaps to the 0.26g HUD (flash_movies/hud.swf) without touching the
// native ones.
//
//   java PatchHudMinimaps IN.swf OUT.swf ENTRIES.tsv
//
// ENTRIES.tsv: key, class, jpeg path, raw 8-bit alpha path, width, height,
// size, posX, posY (tab separated, one map per line). For each map this
//   - adds a DefineBitsJPEG3 image (premultiplied JPEG + zlib alpha, the
//     format of the native minimap_base) bound to a new BitmapData class,
//   - compiles SrpMinimaps.register(props, levels), which adds the map's
//     _minimapProps {size, posX, posY} and _levels bitmap under its key,
//   - calls it at the end of survarium.controls.Minimap's constructor.
// The HUD looks both tables up by the project's project_name (setProps
// value.map); every native key keeps its entry. Fails if any method other
// than the constructor changes, or if the SWF was already patched.
import java.io.*;
import java.nio.file.*;
import java.util.*;
import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.avm2.AVM2Code;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.avm2.parser.script.ActionScript3Parser;
import com.jpexs.decompiler.flash.abc.types.MethodBody;
import com.jpexs.decompiler.flash.abc.types.Multiname;
import com.jpexs.decompiler.flash.tags.*;
import com.jpexs.helpers.ByteArrayRange;

public class PatchHudMinimaps {
    private static final String MINIMAP = "survarium.controls.Minimap";
    private static final String REGISTRY = "SrpMinimaps";

    private static int propertyName(ABC abc, AVM2Code code, String name) throws IOException {
        for (AVM2Instruction ins : code.code) {
            int op = ins.definition.instructionCode;
            if ((op == 0x66 || op == 0x61 || op == 0x68) && ins.operands.length > 0) {
                Multiname m = abc.constants.getMultiname(ins.operands[0]);
                if (m.name_index > 0 && name.equals(abc.constants.getString(m.name_index)))
                    return ins.operands[0];
            }
        }
        throw new IOException("Minimap constructor does not access " + name);
    }

    private static String number(String s) {
        double v = Double.parseDouble(s);
        if (!Double.isFinite(v)) throw new IllegalArgumentException("non-finite " + s);
        return Double.toString(v);
    }

    public static void main(String[] args) throws Exception {
        SWF swf;
        try (InputStream in = Files.newInputStream(Path.of(args[0]))) { swf = new SWF(in, true); }
        List<String[]> entries = new ArrayList<>();
        for (String line : Files.readAllLines(Path.of(args[2])))
            if (!line.isBlank()) entries.add(line.split("\t"));
        int abcIndex = -1;
        for (int i = 0; i < swf.getAbcList().size(); i++) {
            ABC a = swf.getAbcList().get(i).getABC();
            if (a.findClassByName(REGISTRY) >= 0) throw new IOException("Already patched");
            if (a.findClassByName(MINIMAP) >= 0) {
                if (abcIndex >= 0) throw new IOException("Multiple Minimap classes");
                abcIndex = i;
            }
        }
        if (abcIndex < 0) throw new IOException("Missing Minimap class");
        ABC abc = swf.getAbcList().get(abcIndex).getABC();
        Tag abcTag = (Tag) abc.parentTag;
        List<byte[]> originals = new ArrayList<>();
        for (MethodBody body : abc.bodies) originals.add(body.getCodeBytes().clone());
        swf.getAbcIndex().selectAbc(abc);
        StringBuilder register = new StringBuilder(
            "package {\n public class " + REGISTRY + " {\n"
            + "  public static function register(props:Object, levels:Object):void {\n");
        for (String[] e : entries) {
            String key = e[0], cls = e[1];
            if (!key.matches("[a-z0-9_]+") || !cls.matches("[A-Za-z_][A-Za-z0-9_]*"))
                throw new IOException("Bad key or class " + key + " " + cls);
            int w = Integer.parseInt(e[4]), h = Integer.parseInt(e[5]);
            String source = "package {\n import flash.display.BitmapData;\n"
                + " public dynamic class " + cls + " extends BitmapData {\n"
                + "  public function " + cls + "(w:int = " + w + ", h:int = " + h + ") { super(w, h); }\n }\n}\n";
            ActionScript3Parser.compile(source, abc, swf.getAbcIndex(), cls + ".as",
                abc.instance_info.size(), abc.script_info.size(), false, "");
            register.append("   if (!props[\"" + key + "\"]) {\n")
                .append("    props[\"" + key + "\"] = {\"size\":" + number(e[6]) + ", \"posX\":" + number(e[7])
                        + ", \"posY\":" + number(e[8]) + "};\n")
                .append("    levels[\"" + key + "\"] = new " + cls + "();\n   }\n");
        }
        register.append("  }\n }\n}\n");
        ActionScript3Parser.compile(register.toString(), abc, swf.getAbcIndex(), REGISTRY + ".as",
            abc.instance_info.size(), abc.script_info.size(), false, "");
        for (int i = 0; i < originals.size(); i++)
            if (!Arrays.equals(originals.get(i), abc.bodies.get(i).getCodeBytes()))
                throw new IOException("Compiler changed existing method " + i);

        // Minimap constructor: ... returnvoid  ->  ... SrpMinimaps.register(
        // this._minimapProps, this._levels); returnvoid
        int minimap = abc.findClassByName(MINIMAP);
        MethodBody iinit = abc.findBody(abc.instance_info.get(minimap).iinit_index);
        AVM2Code code = iinit.getCode();
        int end = code.code.size() - 1;
        if (code.code.get(end).definition.instructionCode != 0x47 || iinit.exceptions.length != 0)
            throw new IOException("Unexpected Minimap constructor layout");
        int props = propertyName(abc, code, "_minimapProps");
        int levels = propertyName(abc, code, "_levels");
        int registry = abc.constants.getPublicQnameId(REGISTRY, true);
        int method = abc.constants.getPublicQnameId("register", true);
        int at = end;
        code.insertInstruction(at++, new AVM2Instruction(0, 0x60, new int[]{registry}), iinit);
        code.insertInstruction(at++, new AVM2Instruction(0, 0xd0, new int[]{}), iinit);
        code.insertInstruction(at++, new AVM2Instruction(0, 0x66, new int[]{props}), iinit);
        code.insertInstruction(at++, new AVM2Instruction(0, 0xd0, new int[]{}), iinit);
        code.insertInstruction(at++, new AVM2Instruction(0, 0x66, new int[]{levels}), iinit);
        code.insertInstruction(at, new AVM2Instruction(0, 0x4f, new int[]{method, 2}), iinit);
        iinit.max_stack = Math.max(iinit.max_stack, 3);
        iinit.setCode(code);
        iinit.getCode().checkValidOffsets(iinit);
        abcTag.setModified(true);

        // Images before the Minimap ABC; classes bound in the SymbolClass.
        SymbolClassTag symbols = null;
        for (Tag t : swf.getTags())
            if (t instanceof SymbolClassTag) {
                if (symbols != null) throw new IOException("Multiple SymbolClass tags");
                symbols = (SymbolClassTag) t;
            }
        if (symbols == null) throw new IOException("Missing SymbolClass");
        int position = swf.getTags().indexOf(abcTag);
        for (String[] e : entries) {
            DefineBitsJPEG3Tag image = new DefineBitsJPEG3Tag(swf);
            int id = swf.getNextCharacterId();
            image.setCharacterId(id);
            image.imageData = new ByteArrayRange(Files.readAllBytes(Path.of(e[2])));
            image.setImageAlpha(Files.readAllBytes(Path.of(e[3])));
            image.setTimelined(swf);
            image.setModified(true);
            swf.addTag(position++, image);
            symbols.tags.add(id);
            symbols.names.add(e[1]);
            swf.updateCharacters();
        }
        symbols.setModified(true);
        swf.setModified(true);
        try (OutputStream out = Files.newOutputStream(Path.of(args[1]))) { swf.saveTo(out); }

        // Re-read: every pre-existing method body except the constructor is
        // byte-identical, the registry exists and each image decodes.
        try (InputStream in = Files.newInputStream(Path.of(args[1]))) {
            SWF saved = new SWF(in, true);
            ABC s = saved.getAbcList().get(abcIndex).getABC();
            if (s.findClassByName(REGISTRY) < 0) throw new IOException("Missing registry");
            for (int i = 0; i < originals.size(); i++)
                if (abc.bodies.get(i) != iinit && !Arrays.equals(originals.get(i), s.bodies.get(i).getCodeBytes()))
                    throw new IOException("Unrelated method changed " + i);
            Map<String, Integer> bound = new HashMap<>();
            for (Tag t : saved.getTags())
                if (t instanceof SymbolClassTag) {
                    SymbolClassTag st = (SymbolClassTag) t;
                    for (int i = 0; i < st.tags.size(); i++) bound.put(st.names.get(i), st.tags.get(i));
                }
            for (String[] e : entries) {
                Integer id = bound.get(e[1]);
                if (id == null) throw new IOException("Unbound " + e[1]);
                DefineBitsJPEG3Tag img = (DefineBitsJPEG3Tag) saved.getCharacter(id);
                if (img.getImageDimension().width != Integer.parseInt(e[4])
                    || img.getImageAlpha().length != Integer.parseInt(e[4]) * Integer.parseInt(e[5]))
                    throw new IOException("Image " + e[1] + " did not round-trip");
            }
        }
        System.out.println("HUD minimaps added: " + entries.size() + "; unrelated methods preserved");
    }
}
