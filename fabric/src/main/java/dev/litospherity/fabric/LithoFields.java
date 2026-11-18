package dev.litospherity.fabric;

import com.mojang.serialization.Codec;
import com.mojang.serialization.MapCodec;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import net.minecraft.util.KeyDispatchDataCodec;
import net.minecraft.world.level.levelgen.DensityFunction;

/**
 * Ф3: функции плотности Litospherity.
 *
 * <p>В датапаке узел без сида: {@code {"type":"litospherity:field","field":"continents"}}. Сид и режим
 * приходят при создании {@code RandomState} (миксин {@code RandomStateMixin}), и в {@code mapAll} узел
 * заменяется связанным с генератором ядра. Значение то же, что в Horizon: {@code field_f32(x, z)}.
 */
public final class LithoFields {
    private LithoFields() {}

    /** Порядок как Field::ALL в litho-core. */
    public static final List<String> FIELDS = List.of(
            "continents", "erosion", "ridges", "temperature", "vegetation",
            "lava", "offset", "pv", "factor", "jaggedness");

    /** Версия генератора: 2 = v1-dev (как паки Horizon). */
    public static final int VERSION = 2;

    /** Сид и режим текущего создаваемого RandomState (0 научный, 1 геймплейный). */
    public record Binding(long seed, int mode) {}

    public static final ThreadLocal<Binding> CURRENT = new ThreadLocal<>();

    private static final Map<Binding, LithoNative.Generator> GENERATORS = new ConcurrentHashMap<>();

    static LithoNative.Generator generator(Binding b) {
        return GENERATORS.computeIfAbsent(b, k -> {
            LithoMod.LOG.info("Litospherity: генератор сид {} режим {} версия v1-dev",
                    k.seed(), k.mode() == 0 ? "scientific" : "gameplay");
            return LithoNative.get().newGenerator(k.seed(), k.mode(), VERSION);
        });
    }

    /** Поле ядра в колонке (x, z). */
    public record Field(String name, LithoNative.Generator generator) implements DensityFunction.SimpleFunction {
        public static final MapCodec<Field> MAP_CODEC = Codec.STRING
                .validate(n -> FIELDS.contains(n)
                        ? com.mojang.serialization.DataResult.success(n)
                        : com.mojang.serialization.DataResult.error(() -> "неизвестное поле Litospherity: " + n))
                .fieldOf("field")
                .xmap(n -> new Field(n, null), Field::name);
        public static final KeyDispatchDataCodec<Field> CODEC = KeyDispatchDataCodec.of(MAP_CODEC);

        @Override
        public double compute(FunctionContext ctx) {
            if (generator == null) return 0.0;
            return generator.field(FIELDS.indexOf(name), ctx.blockX(), ctx.blockZ());
        }

        @Override
        public DensityFunction mapAll(Visitor visitor) {
            Binding b = CURRENT.get();
            if (generator == null && b != null) return visitor.apply(new Field(name, generator(b)));
            return visitor.apply(this);
        }

        @Override public double minValue() { return -64.0; }
        @Override public double maxValue() { return 64.0; }
        @Override public KeyDispatchDataCodec<? extends DensityFunction> codec() { return CODEC; }
    }

    /**
     * Лава аквифера (Д-097): роутер {@code lava} зовётся в координатах ячейки аквифера
     * (64 × 40 × 64), как хук Horizon на {@code minecraft:aquifer_lava}.
     */
    public record LavaAquifer(LithoNative.Generator generator) implements DensityFunction.SimpleFunction {
        public static final MapCodec<LavaAquifer> MAP_CODEC = MapCodec.unit(() -> new LavaAquifer(null));
        public static final KeyDispatchDataCodec<LavaAquifer> CODEC = KeyDispatchDataCodec.of(MAP_CODEC);

        @Override
        public double compute(FunctionContext ctx) {
            if (generator == null) return 0.0;
            return generator.lavaAquifer(ctx.blockX(), ctx.blockY(), ctx.blockZ());
        }

        @Override
        public DensityFunction mapAll(Visitor visitor) {
            Binding b = CURRENT.get();
            if (generator == null && b != null) return visitor.apply(new LavaAquifer(generator(b)));
            return visitor.apply(this);
        }

        @Override public double minValue() { return -4.0; }
        @Override public double maxValue() { return 4.0; }
        @Override public KeyDispatchDataCodec<? extends DensityFunction> codec() { return CODEC; }
    }
}
