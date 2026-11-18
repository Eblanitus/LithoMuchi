package dev.litospherity.fabric.mixin;

import dev.litospherity.fabric.LithoFields;
import net.minecraft.core.HolderGetter;
import net.minecraft.world.level.levelgen.NoiseGeneratorSettings;
import net.minecraft.world.level.levelgen.RandomState;
import net.minecraft.world.level.levelgen.synth.NormalNoise;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

/**
 * Ф3: передаёт сид мира узлам Litospherity. Режим — по высоте мира: научный мир 512 блоков
 * (Д-113, пресет litospherity:scientific), всё остальное — геймплейный.
 */
@Mixin(RandomState.class)
public abstract class RandomStateMixin {
    @Inject(method = "create(Lnet/minecraft/world/level/levelgen/NoiseGeneratorSettings;Lnet/minecraft/core/HolderGetter;J)Lnet/minecraft/world/level/levelgen/RandomState;",
            at = @At("HEAD"))
    private static void litospherity$bind(NoiseGeneratorSettings settings, HolderGetter<NormalNoise.NoiseParameters> noises,
                                          long seed, CallbackInfoReturnable<RandomState> cir) {
        int mode = settings.noiseSettings().height() >= 512 ? 0 : 1;
        LithoFields.CURRENT.set(new LithoFields.Binding(seed, mode));
    }

    @Inject(method = "create(Lnet/minecraft/world/level/levelgen/NoiseGeneratorSettings;Lnet/minecraft/core/HolderGetter;J)Lnet/minecraft/world/level/levelgen/RandomState;",
            at = @At("RETURN"))
    private static void litospherity$unbind(NoiseGeneratorSettings settings, HolderGetter<NormalNoise.NoiseParameters> noises,
                                            long seed, CallbackInfoReturnable<RandomState> cir) {
        LithoFields.CURRENT.remove();
    }
}
