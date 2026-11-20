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
            "lava", "offset", "pv", "factor", "jaggedness", "calm");

    /** Версия генератора: 2 = v1-dev (как паки Horizon). */
    public static final int VERSION_V1_DEV = 2;
    /** Ф6-В1: вода выше уровня моря; пресеты Litospherity на Fabric. */
    public static final int VERSION_R2 = 3;

    /** Сид, режим (0 научный, 1 геймплейный) и версия генератора ядра. */
    public record Binding(long seed, int mode, int version) {}

    /** Режим мира по RandomState (0 научный, 1 геймплейный); заполняет миксин. */
    public static final Map<RandomState, Integer> MODES = Collections.synchronizedMap(new WeakHashMap<>());

    /** Ф6-В1: версия генератора по RandomState; заполняет {@link LithoWorlds}. */
    public static final Map<RandomState, Integer> VERSIONS = Collections.synchronizedMap(new WeakHashMap<>());

    /** Привязка Overworld для фич (Ф6-В3/В4): у сервера один такой мир. */
    private static volatile Binding world;

    static void setWorld(Binding binding) {
        world = binding;
    }

    /** Привязка мира для фич; {@code null} до загрузки Overworld. */
    public static Binding world() {
        return world;
    }

    private static final Map<Binding, LithoNative.Generator> GENERATORS = new ConcurrentHashMap<>();

    static LithoNative.Generator generator(Binding b) {
        return GENERATORS.computeIfAbsent(b, k -> {
            LithoMod.LOG.info("Litospherity: генератор сид {} режим {} версия {}",
                    k.seed(), k.mode() == 0 ? "scientific" : "gameplay",
                    k.version() == VERSION_R2 ? "v1-dev-r2" : "v1-dev");
            return LithoNative.get().newGenerator(k.seed(), k.mode(), k.version());
        });
    }

    /** Сид и режим из контекста компиляции (анонимный класс RandomState хранит ссылку на него). */
    static Binding binding(DensityFunction.CompileContext ctx) {
        try {
            Field outer = ctx.getClass().getDeclaredField("this$0");
            outer.setAccessible(true);
            RandomState state = (RandomState) outer.get(ctx);
            return binding(state);
        } catch (ReflectiveOperationException | ClassCastException e) {
            long seed = ctx.createRandom(Identifier.fromNamespaceAndPath(LithoMod.MOD_ID, "seed")).nextLong();
            LithoMod.LOG.warn("Litospherity: сид мира недоступен ({}), беру производный {}", e, seed);
            return new Binding(seed, 1, VERSION_V1_DEV);
        }
    }

    /** Привязка по RandomState: режим из миксина, версия — из {@link LithoWorlds}. */
    public static Binding binding(RandomState state) {
        int mode = MODES.getOrDefault(state, 1);
        int version = VERSIONS.getOrDefault(state, VERSION_V1_DEV);
        return new Binding(state.seed(), mode, version);
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
