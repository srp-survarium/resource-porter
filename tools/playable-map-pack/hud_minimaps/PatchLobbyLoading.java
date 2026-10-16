// SPDX-License-Identifier: GPL-3.0-or-later
// Add loading screens to the 0.26g lobby (flash_movies/lobby_menu.swf)
// without touching the native ones.
//
//   java PatchLobbyLoading IN.swf OUT.swf ENTRIES.tsv NATIVE_COUNT
//
// ENTRIES.tsv: class, jpeg path, raw 8-bit alpha path, width, height (tab
// separated, one image per line, in loading_screen_id order). The engine
// calls root.set_loading_level(mode.ui.loading_screen_id) and LobbyMenu
// shows _load_images[id] (native ids 0..NATIVE_COUNT-1). For each image this
//   - adds a DefineBitsJPEG3 image bound to a new BitmapData class,
//   - compiles SrpLoadingImages.register(images), which appends the classes,
//   - calls it at the end of LobbyMenu's constructor,
// so image k gets id NATIVE_COUNT + k. The constructor must build
// _load_images from exactly NATIVE_COUNT classes. Fails if any method other
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

public class PatchLobbyLoading {
    private static final String LOBBY = "LobbyMenu";
    private static final String REGISTRY = "SrpLoadingImages";
    private static final String FIELD = "_load_images";

    private static boolean names(ABC abc, AVM2Instruction ins, String name) {
        if (ins.operands.length == 0) return false;
        Multiname m = abc.constants.getMultiname(ins.operands[0]);
        return m.name_index > 0 && name.equals(abc.constants.getString(m.name_index));
    }

    public static void main(String[] args) throws Exception {
        SWF swf;
        try (InputStream in = Files.newInputStream(Path.of(args[0]))) { swf = new SWF(in, true); }
        List<String[]> entries = new ArrayList<>();
        for (String line : Files.readAllLines(Path.of(args[2])))
            if (!line.isBlank()) entries.add(line.split("\t"));
        int nativeCount = Integer.parseInt(args[3]);
        int abcIndex = -1;
        for (int i = 0; i < swf.getAbcList().size(); i++) {
            ABC a = swf.getAbcList().get(i).getABC();
            if (a.findClassByName(REGISTRY) >= 0) throw new IOException("Already patched");
            if (a.findClassByName(LOBBY) >= 0) {
                if (abcIndex >= 0) throw new IOException("Multiple LobbyMenu classes");
                abcIndex = i;
            }
        }
        if (abcIndex < 0) throw new IOException("Missing LobbyMenu class");
        ABC abc = swf.getAbcList().get(abcIndex).getABC();
        Tag abcTag = (Tag) abc.parentTag;
        int lobby = abc.findClassByName(LOBBY);
        MethodBody iinit = abc.findBody(abc.instance_info.get(lobby).iinit_index);
        AVM2Code code = iinit.getCode();
        // The field initialiser: newarray NATIVE_COUNT; initproperty _load_images.
        int field = -1;
        for (int i = 1; i < code.code.size(); i++) {
            AVM2Instruction ins = code.code.get(i);
            int op = ins.definition.instructionCode;
            if ((op == 0x68 || op == 0x61) && names(abc, ins, FIELD)) {
                AVM2Instruction prev = code.code.get(i - 1);
                if (prev.definition.instructionCode != 0x56 || prev.operands[0] != nativeCount)
                    throw new IOException(FIELD + " is not built from " + nativeCount + " images");
                if (field >= 0) throw new IOException("Two " + FIELD + " initialisers");
                field = ins.operands[0];
            }
        }
        if (field < 0) throw new IOException("LobbyMenu constructor does not set " + FIELD);
        List<byte[]> originals = new ArrayList<>();
        for (MethodBody body : abc.bodies) originals.add(body.getCodeBytes().clone());
        swf.getAbcIndex().selectAbc(abc);
        StringBuilder register = new StringBuilder(
            "package {\n public class " + REGISTRY + " {\n"
            + "  public static function register(images:Array):void {\n"
            + "   if (images.length != " + nativeCount + ") return;\n");
        for (String[] e : entries) {
            String cls = e[0];
            if (!cls.matches("[A-Za-z_][A-Za-z0-9_]*")) throw new IOException("Bad class " + cls);
            int w = Integer.parseInt(e[3]), h = Integer.parseInt(e[4]);
            String source = "package {\n import flash.display.BitmapData;\n"
                + " public dynamic class " + cls + " extends BitmapData {\n"
                + "  public function " + cls + "(w:int = " + w + ", h:int = " + h + ") { super(w, h); }\n }\n}\n";
            ActionScript3Parser.compile(source, abc, swf.getAbcIndex(), cls + ".as",
                abc.instance_info.size(), abc.script_info.size(), false, "");
            register.append("   images.push(" + cls + ");\n");
        }
        register.append("  }\n }\n}\n");
        ActionScript3Parser.compile(register.toString(), abc, swf.getAbcIndex(), REGISTRY + ".as",
            abc.instance_info.size(), abc.script_info.size(), false, "");
        for (int i = 0; i < originals.size(); i++)
            if (!Arrays.equals(originals.get(i), abc.bodies.get(i).getCodeBytes()))
                throw new IOException("Compiler changed existing method " + i);

        // LobbyMenu constructor: ... returnvoid -> ...
        // SrpLoadingImages.register(this._load_images); returnvoid
        code = iinit.getCode();
        int end = code.code.size() - 1;
        if (code.code.get(end).definition.instructionCode != 0x47 || iinit.exceptions.length != 0)
            throw new IOException("Unexpected LobbyMenu constructor layout");
        int registry = abc.constants.getPublicQnameId(REGISTRY, true);
        int method = abc.constants.getPublicQnameId("register", true);
        int at = end;
        code.insertInstruction(at++, new AVM2Instruction(0, 0x60, new int[]{registry}), iinit);
        code.insertInstruction(at++, new AVM2Instruction(0, 0xd0, new int[]{}), iinit);
        code.insertInstruction(at++, new AVM2Instruction(0, 0x66, new int[]{field}), iinit);
        code.insertInstruction(at, new AVM2Instruction(0, 0x4f, new int[]{method, 1}), iinit);
        iinit.max_stack = Math.max(iinit.max_stack, 2);
        iinit.setCode(code);
        iinit.getCode().checkValidOffsets(iinit);
        abcTag.setModified(true);

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
            image.imageData = new ByteArrayRange(Files.readAllBytes(Path.of(e[1])));
            image.setImageAlpha(Files.readAllBytes(Path.of(e[2])));
            image.setTimelined(swf);
            image.setModified(true);
            swf.addTag(position++, image);
            symbols.tags.add(id);
            symbols.names.add(e[0]);
            swf.updateCharacters();
        }
        symbols.setModified(true);
        swf.setModified(true);
        try (OutputStream out = Files.newOutputStream(Path.of(args[1]))) { swf.saveTo(out); }

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
                Integer id = bound.get(e[0]);
                if (id == null) throw new IOException("Unbound " + e[0]);
                DefineBitsJPEG3Tag img = (DefineBitsJPEG3Tag) saved.getCharacter(id);
                if (img.getImageDimension().width != Integer.parseInt(e[3])
                    || img.getImageDimension().height != Integer.parseInt(e[4]))
                    throw new IOException("Image " + e[0] + " did not round-trip");
            }
        }
        for (int k = 0; k < entries.size(); k++)
            System.out.println("loading_screen_id " + (nativeCount + k) + " = " + entries.get(k)[0]);
    }
}
