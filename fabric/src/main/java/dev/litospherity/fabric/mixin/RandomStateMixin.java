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
 * Ф3: режим мира для узлов Litospherity. Научный мир — 512 блоков (Д-113, пресет
 * litospherity:scientific), всё остальное — геймплейный.
 */
@Mixin(RandomState.class)
public abstract class RandomStateMixin {
    @Inject(method = "create(Lnet/minecraft/core/HolderGetter;JLnet/minecraft/world/level/levelgen/NoiseGeneratorSettings;)Lnet/minecraft/world/level/levelgen/RandomState;",
            at = @At("RETURN"))
    private static void litospherity$mode(HolderGetter<NormalNoise> noises, long seed, NoiseGeneratorSettings settings,
                                          CallbackInfoReturnable<RandomState> cir) {
        LithoFields.MODES.put(cir.getReturnValue(), settings.noiseSettings().height() >= 512 ? 0 : 1);
    }
}
