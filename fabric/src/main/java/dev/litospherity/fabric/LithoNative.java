package dev.litospherity.fabric;

import java.io.IOException;
import java.io.InputStream;
import java.lang.foreign.Arena;
import java.lang.foreign.FunctionDescriptor;
import java.lang.foreign.Linker;
import java.lang.foreign.MemorySegment;
import java.lang.foreign.SymbolLookup;
import java.lang.foreign.ValueLayout;
import java.lang.invoke.MethodHandle;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.Locale;

/**
 * Обёртка над litho-ffi (crates/litho-ffi) через Panama FFM.
 * Библиотека лежит в jar: natives/&lt;os&gt;-&lt;arch&gt;/&lt;имя&gt;, при запуске копируется во временный файл.
 */
public final class LithoNative {
    public static final int EXPECTED_ABI = 3;
    private static volatile LithoNative instance;

    private final int fieldCount;
    private final MethodHandle abiVersion, fieldCountH, coreVersion, genNew, genFree, fieldsBatch, lavaAquifer;
    private final MethodHandle waterBatch2;

    public static LithoNative get() {
        LithoNative local = instance;
        if (local == null) {
            synchronized (LithoNative.class) {
                local = instance;
                if (local == null) instance = local = new LithoNative(extract());
            }
        }
        return local;
    }

    public static String platform() {
        String os = System.getProperty("os.name").toLowerCase(Locale.ROOT);
        String arch = System.getProperty("os.arch").toLowerCase(Locale.ROOT);
        String o = os.contains("win") ? "windows" : os.contains("mac") ? "macos" : "linux";
        String a = (arch.equals("aarch64") || arch.equals("arm64")) ? "aarch64" : "x86_64";
        return o + "-" + a;
    }

    private static String libName() {
        String p = platform();
        if (p.startsWith("windows")) return "litho_ffi.dll";
        if (p.startsWith("macos")) return "liblitho_ffi.dylib";
        return "liblitho_ffi.so";
    }

    private static Path extract() {
        String resource = "/natives/" + platform() + "/" + libName();
        try (InputStream in = LithoNative.class.getResourceAsStream(resource)) {
            if (in == null) throw new IllegalStateException("в jar нет " + resource);
            Path file = Files.createTempFile("litho_ffi", libName());
            Files.copy(in, file, StandardCopyOption.REPLACE_EXISTING);
            file.toFile().deleteOnExit();
            return file;
        } catch (IOException e) {
            throw new IllegalStateException("не удалось распаковать " + resource, e);
        }
    }

    private LithoNative(Path library) {
        Linker linker = Linker.nativeLinker();
        SymbolLookup lib = SymbolLookup.libraryLookup(library, Arena.global());
        abiVersion = linker.downcallHandle(lib.findOrThrow("litho_abi_version"), FunctionDescriptor.of(ValueLayout.JAVA_INT));
        fieldCountH = linker.downcallHandle(lib.findOrThrow("litho_field_count"), FunctionDescriptor.of(ValueLayout.JAVA_INT));
        coreVersion = linker.downcallHandle(lib.findOrThrow("litho_core_version"), FunctionDescriptor.of(ValueLayout.ADDRESS));
        genNew = linker.downcallHandle(lib.findOrThrow("litho_generator_new"),
                FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.JAVA_INT, ValueLayout.JAVA_INT));
        genFree = linker.downcallHandle(lib.findOrThrow("litho_generator_free"), FunctionDescriptor.ofVoid(ValueLayout.ADDRESS));
        fieldsBatch = linker.downcallHandle(lib.findOrThrow("litho_fields_batch"),
                FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS));
        lavaAquifer = linker.downcallHandle(lib.findOrThrow("litho_lava_aquifer"),
                FunctionDescriptor.of(ValueLayout.JAVA_DOUBLE, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.JAVA_LONG, ValueLayout.JAVA_LONG));
        // ВО2-2 (ABI 3): вода с потоком, материалами и видом водоёма.
        waterBatch2 = linker.downcallHandle(lib.findOrThrow("litho_water_batch2"),
                FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                        ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS));
        int abi = abiVersion();
        if (abi != EXPECTED_ABI) throw new IllegalStateException("ABI ядра " + abi + ", ожидался " + EXPECTED_ABI);
        try {
            fieldCount = (int) fieldCountH.invokeExact();
        } catch (Throwable t) {
            throw new IllegalStateException(t);
        }
    }

    public int abiVersion() {
        try { return (int) abiVersion.invokeExact(); } catch (Throwable t) { throw new IllegalStateException(t); }
    }

    public int fieldCount() { return fieldCount; }

    public String coreVersion() {
        try {
            MemorySegment p = (MemorySegment) coreVersion.invokeExact();
            return p.reinterpret(64).getString(0);
        } catch (Throwable t) { throw new IllegalStateException(t); }
    }

    /** mode: 0 научный, 1 геймплейный; version: 0 v0, 1 v1, 2 v1-dev, 3 v1-dev-r2, 4 v1-dev-w. */
    public Generator newGenerator(long seed, int mode, int version) {
        try {
            MemorySegment p = (MemorySegment) genNew.invokeExact(seed, mode, version);
            if (p.address() == 0) throw new IllegalArgumentException("ядро отказало: mode " + mode + ", version " + version);
            return new Generator(p);
        } catch (RuntimeException e) { throw e; } catch (Throwable t) { throw new IllegalStateException(t); }
    }

    /** Генератор ядра. Потокобезопасный; закрывать один раз. */
    public final class Generator implements AutoCloseable {
        private MemorySegment ptr;
        private Generator(MemorySegment ptr) { this.ptr = ptr; }

        /** xz: пары (x, z) в блоках. Ответ: fieldCount значений на колонку подряд (порядок Field::ALL). */
        public float[] fields(double[] xz) {
            int n = xz.length / 2;
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment in = arena.allocateFrom(ValueLayout.JAVA_DOUBLE, xz);
                MemorySegment out = arena.allocate(ValueLayout.JAVA_FLOAT, (long) n * fieldCount);
                int code = (int) fieldsBatch.invokeExact(ptr, in, (long) n, out);
                if (code != 0) throw new IllegalStateException("litho_fields_batch вернул " + code);
                return out.toArray(ValueLayout.JAVA_FLOAT);
            } catch (RuntimeException e) { throw e; } catch (Throwable t) { throw new IllegalStateException(t); }
        }

        /** Кэш последней колонки на поток: игра спрашивает несколько полей подряд в одной колонке. */
        private final ThreadLocal<Column> column = ThreadLocal.withInitial(Column::new);

        private final class Column {
            final MemorySegment in = Arena.ofAuto().allocate(ValueLayout.JAVA_DOUBLE, 2);
            final MemorySegment out = Arena.ofAuto().allocate(ValueLayout.JAVA_FLOAT, fieldCount);
            int x = Integer.MIN_VALUE, z = Integer.MIN_VALUE;
            boolean valid;
        }

        /** Одно поле (индекс как Field::ALL) в колонке блока (x, z). */
        public double field(int index, int x, int z) {
            Column c = column.get();
            if (!c.valid || c.x != x || c.z != z) {
                c.in.setAtIndex(ValueLayout.JAVA_DOUBLE, 0, x);
                c.in.setAtIndex(ValueLayout.JAVA_DOUBLE, 1, z);
                int code;
                try {
                    code = (int) fieldsBatch.invokeExact(ptr, c.in, 1L, c.out);
                } catch (Throwable t) { throw new IllegalStateException(t); }
                if (code != 0) return 0.0;
                c.x = x; c.z = z; c.valid = true;
            }
            return c.out.getAtIndex(ValueLayout.JAVA_FLOAT, index);
        }

        /**
         * ВО2-2: вода колонок (x, z). Ответ — уровень (верхний блок воды + 1,
         * {@code Integer.MIN_VALUE} — воды выше моря нет), дно, код вида и
         * биты {@code info}: поток (0–3), дно (4–7), берег (8–11), вид воды (12–17).
         */
        public WaterColumns water(double[] xs, double[] zs) {
            if (xs.length != zs.length) throw new IllegalArgumentException("xs и zs разной длины");
            int n = xs.length;
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment inX = arena.allocateFrom(ValueLayout.JAVA_DOUBLE, xs);
                MemorySegment inZ = arena.allocateFrom(ValueLayout.JAVA_DOUBLE, zs);
                MemorySegment level = arena.allocate(ValueLayout.JAVA_INT, n);
                MemorySegment bed = arena.allocate(ValueLayout.JAVA_INT, n);
                MemorySegment kind = arena.allocate(ValueLayout.JAVA_BYTE, n);
                MemorySegment info = arena.allocate(ValueLayout.JAVA_INT, n);
                int code = (int) waterBatch2.invokeExact(ptr, inX, inZ, (long) n, level, bed, kind, info);
                if (code != 0) throw new IllegalStateException("litho_water_batch2 вернул " + code);
                return new WaterColumns(
                        level.toArray(ValueLayout.JAVA_INT),
                        bed.toArray(ValueLayout.JAVA_INT),
                        kind.toArray(ValueLayout.JAVA_BYTE),
                        info.toArray(ValueLayout.JAVA_INT));
            } catch (RuntimeException e) {
                throw e;
            } catch (Throwable t) {
                throw new IllegalStateException(t);
            }
        }

        /** Лава в ячейке аквифера; 0 при ошибке ядра. */
        public double lavaAquifer(int cx, int cy, int cz) {
            try {
                double v = (double) lavaAquifer.invokeExact(ptr, (long) cx, (long) cy, (long) cz);
                return Double.isNaN(v) ? 0.0 : v;
            } catch (Throwable t) { throw new IllegalStateException(t); }
        }

        @Override
        public synchronized void close() {
            if (ptr == null) return;
            try { genFree.invokeExact(ptr); } catch (Throwable t) { throw new IllegalStateException(t); }
            ptr = null;
        }
    }

    /** ВО2-2: ответ {@code litho_water_batch2} по колонкам пакета. */
    public record WaterColumns(int[] level, int[] bed, byte[] kind, int[] info) {
        /** Воды выше моря нет. */
        public static final int NONE = Integer.MIN_VALUE;

        public static int flow(int info) { return info & 0xF; }
        public static int bedMat(int info) { return (info >>> 4) & 0xF; }
        public static int shoreMat(int info) { return (info >>> 8) & 0xF; }
        public static int body(int info) { return (info >>> 12) & 0x3F; }
    }
}
