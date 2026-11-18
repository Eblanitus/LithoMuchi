package dev.litospherity.fabric;

import com.mojang.serialization.Codec;
import com.mojang.serialization.DataResult;
import com.mojang.serialization.MapCodec;
import java.lang.reflect.Field;
import java.util.Collections;
import java.util.List;
import java.util.Map;
import java.util.WeakHashMap;
import java.util.concurrent.ConcurrentHashMap;
import net.minecraft.resources.Identifier;
import net.minecraft.util.Interval;
import net.minecraft.world.level.levelgen.RandomState;
import net.minecraft.world.level.levelgen.densityfunction.DensityBuffer;
import net.minecraft.world.level.levelgen.densityfunction.DensityFunction;
import net.minecraft.world.level.levelgen.densityfunction.DensitySampler;
import net.minecraft.world.level.levelgen.densityfunction.DensityVolume;
import net.minecraft.world.level.levelgen.densityfunction.DfRewriteRule;
import net.minecraft.world.level.levelgen.densityfunction.SamplerContext;

/**
 * Ф3: функции плотности Litospherity для Minecraft 26.3 (компилируемые сэмплеры).
 *
 * <p>Узел датапака: {@code {"type":"litospherity:field","field":"continents"}} — то же, что шум
 * {@code litospherity:v1-dev/<режим>/<поле>} в Horizon: {@code field_f32(x, z)} ядра. Сид берётся из
 * {@code RandomState} при компиляции сэмплера, режим — из высоты мира (миксин {@code RandomStateMixin}).
 */
public final class LithoFields {
    private LithoFields() {}

    /** Порядок как Field::ALL в litho-core. */
    public static final List<String> FIELDS = List.of(
            "continents", "erosion", "ridges", "temperature", "vegetation",
            "lava", "offset", "pv", "factor", "jaggedness");

    /** Версия генератора: 2 = v1-dev (как паки Horizon). */
    public static final int VERSION = 2;

    public record Binding(long seed, int mode) {}

    /** Режим мира по RandomState (0 научный, 1 геймплейный); заполняет миксин. */
    public static final Map<RandomState, Integer> MODES = Collections.synchronizedMap(new WeakHashMap<>());

    private static final Map<Binding, LithoNative.Generator> GENERATORS = new ConcurrentHashMap<>();

    static LithoNative.Generator generator(Binding b) {
        return GENERATORS.computeIfAbsent(b, k -> {
            LithoMod.LOG.info("Litospherity: генератор сид {} режим {} версия v1-dev",
                    k.seed(), k.mode() == 0 ? "scientific" : "gameplay");
            return LithoNative.get().newGenerator(k.seed(), k.mode(), VERSION);
        });
    }

    /** Сид и режим из контекста компиляции (анонимный класс RandomState хранит ссылку на него). */
    static Binding binding(DensityFunction.CompileContext ctx) {
        try {
            Field outer = ctx.getClass().getDeclaredField("this$0");
            outer.setAccessible(true);
            RandomState state = (RandomState) outer.get(ctx);
            return new Binding(state.seed(), MODES.getOrDefault(state, 1));
        } catch (ReflectiveOperationException | ClassCastException e) {
            long seed = ctx.createRandom(Identifier.fromNamespaceAndPath(LithoMod.MOD_ID, "seed")).nextLong();
            LithoMod.LOG.warn("Litospherity: сид мира недоступен ({}), беру производный {}", e, seed);
            return new Binding(seed, 1);
        }
    }

    private abstract static class Sampler implements DensitySampler {
        @Override
        public void sampleVolume(SamplerContext ctx, DensityBuffer buffer, DensityVolume volume) {
            DensitySampler.sampleVolumeNaive(ctx, buffer, volume, this);
        }
    }

    /** Поле ядра в колонке (x, z). */
    public record ColumnField(String name) implements DensityFunction {
        public static final MapCodec<ColumnField> CODEC = Codec.STRING
                .validate(n -> FIELDS.contains(n) ? DataResult.success(n)
                        : DataResult.error(() -> "неизвестное поле Litospherity: " + n))
                .fieldOf("field")
                .xmap(ColumnField::new, ColumnField::name);

        @Override
        public DensitySampler compileSampler(CompileContext ctx) {
            LithoNative.Generator gen = generator(binding(ctx));
            int index = FIELDS.indexOf(name);
            return new Sampler() {
                @Override
                public float sampleValue(SamplerContext c, int x, int y, int z) {
                    return (float) gen.field(index, x, z);
                }
            };
        }

        @Override public DensityFunction rewriteChildren(DfRewriteRule rule) { return this; }
        @Override public Interval range() { return Interval.of(-64f, 64f); }
        @Override public int domainAxes() { return AXIS_X | AXIS_Z; }
        @Override public MapCodec<? extends DensityFunction> codec() { return CODEC; }
    }

    /**
     * Лава аквифера (Д-097): {@code aquifers.lava} зовётся в координатах ячейки аквифера
     * (64 × 40 × 64), как хук Horizon на {@code minecraft:aquifer_lava}.
     */
    public record LavaAquifer() implements DensityFunction {
        public static final MapCodec<LavaAquifer> CODEC = MapCodec.unit(LavaAquifer::new);

        @Override
        public DensitySampler compileSampler(CompileContext ctx) {
            LithoNative.Generator gen = generator(binding(ctx));
            return new Sampler() {
                @Override
                public float sampleValue(SamplerContext c, int x, int y, int z) {
                    return (float) gen.lavaAquifer(x, y, z);
                }
            };
        }

        @Override public DensityFunction rewriteChildren(DfRewriteRule rule) { return this; }
        @Override public Interval range() { return Interval.of(-4f, 4f); }
        @Override public int domainAxes() { return AXIS_X | AXIS_Y | AXIS_Z; }
        @Override public MapCodec<? extends DensityFunction> codec() { return CODEC; }
    }
}
